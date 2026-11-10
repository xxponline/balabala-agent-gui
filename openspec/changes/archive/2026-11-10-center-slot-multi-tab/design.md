# Design

## Context

中槽已在 `DockLayout::tabs()` 内（`shell/mod.rs`），但只注册了唯一面板
`TerminalPanel`；gpui-base 的 dock 原生支持多 tab——`add_panel_view(Center)`
经 `InsertTarget::Tabs{ix:None}` 尾插并 activate（`edit.rs:202`），
`Panel::closable()`、`Panel::on_removed`、`DockArea::select_panel` 均为现成
trait 钩子。右槽 `WorkspacePropertySource::open(&str)` 是预留的 no-op，
spec 明言"deferring to the future default-open system"——本 change 即该系统。
动机见 proposal.md Why。

约束：
- `agent-shell` 两条绝对条款（main pane 永远渲染 session；失焦是键盘不转发
  唯一例外）被多 tab 打破，spec delta 已在 specs/ 中定案（键盘跟随 active tab）
- 右键/树交互的焦点交回现状是"交回终端面板"（tree 根级 on_click 冒泡），
  多 tab 后须改为"交回 active tab 的面板"
- 前置 change `right-dock-file-tree` 未归档，`file-tree` 主 spec 不存在——
  本 change 的 file-tree delta 只能等它先归档（validate INFO 已确认）

## Goals / Non-Goals

**Goals:**
- 中槽多页签：console 恒为第一签、不可关；viewer 签尾插、可关
- 统一后端接口：封装对象 → handler 注册表分发 → dedup 开签/聚焦
- 首个 handler：只读 Editor 查看 `.rs`（syntect 高亮）
- 右槽左键钩子端到端接入；焦点/键盘模型随 active tab 自洽
- 无头测试守卫：第一签不变量、去重、分发、键盘跟随

**Non-Goals:**
- 非 `.rs` 文件的 handler（含二进制/超大文件守卫）——predicate 放宽即可加
- 可编辑 buffer、保存、脏状态、外部变更监听
- tab 拖拽重排/跨 dock 拖动的约束（v1 接受库默认行为，见 Risks）
- 布局持久化（DockArea dump/load 仍未接，D6 老决定不变）
- tree-sitter 高亮（syntect 足够；升级是 feature 加法）
- 右键菜单接入 open（右键仍走 copy/reveal）

## Decisions

### D1: 后端接口 = `TerminalApp::open_object` + 独立 `src/property_detail_manager/` 模块

```rust
// src/property_detail_manager/mod.rs
pub struct OpenObject { pub path: PathBuf, pub kind: SourceNodeKind } // 中立 DTO
pub trait OpenHandler: 'static {
    fn claims(&self, obj: &OpenObject) -> bool;
    fn create_panel(&self, obj: &OpenObject, cx: &mut Context<TerminalApp>)
        -> Arc<dyn PanelView>;
}
static HANDLERS: &[...] // 静态注册表, v1 仅 rust_viewer
```

`TerminalApp::open_object(&mut self, obj, window, cx)`：
查 dedup（`HashMap<String, PanelId>`，key = path 字符串）→ 命中
`dock_area.select_panel`；未命中 → `claims` 分发 → `create_panel` →
`add_panel_view(Center)` 尾插 → 登记。

- **为何挂在 TerminalApp**：dedup 表与 dock_area 都在 app 手里；与
  `on_sidebar_event` 同款"事件漏斗进单一 model"模式，树/源保持可单测
  （效果由应用层落地，与 `SourceEffect` 分工一致）
- **备选（否）**：独立 `OpenService` 实体——多一层 entity 仪式，v1 无收益；
  注册表未来膨胀到需要热插拔时再拆

### D2: 封装对象由数据源构造，`open` 钩子改签名

`WorkspacePropertySource::open` 从 `fn open(&mut self, item_id: &str)` 改为
返回意图（或直接持 `WeakEntity<TerminalApp>` 调 `open_object`）——
`FileTreeSource` 已持有 app weak（refresh 同款），v1 直接调用，签名不变、
实现从 no-op 变为查权威树得 kind → 构造 `OpenObject` → `open_object`。
目录行：kind=Dir，`open_object` 内 handler 不认领 → 天然 no-op（树库内置
展开切换不受影响）。

- **key = 绝对路径字符串**：与 `SourceNode.id` 同一身份体系，树 id 来自
  扫描的绝对路径，无重复变体；canonical 化收益（symlink）v1 不值一提

### D3: console 第一签不变量 = 尾插 + closable=false（不挡拖拽）

`add_panel_view(Center)` 永远追加到 tab 组尾部 → console 天然 index 0；
`TerminalPanel::closable() -> false` 已有 → 关不掉。**不**额外禁用 tab
拖拽（库内建 `TabGroupConstraints`）：v1 用户把 viewer 拖到 console 前
不破坏"console 存在且唯一"的语义，只破坏位置；若将来 spec 要钉死位置，
再加 `draggable` 约束（备选记录于此）。

### D4: viewer 面板 = `EditorPanel` 实体，内嵌 gpui-component `Editor`

```rust
// src/property_detail_manager/editor_panel.rs
pub struct EditorPanel {
    state: Entity<EditorState>,   // EditorState::new(window, cx).language("rust")
    title: String,                // 文件名 (panel title/tab)
    app: WeakEntity<TerminalApp>, // on_removed 摘 dedup
}
impl Panel for EditorPanel { panel_name "viewer"; closable true; title_bar false }
```

- 内容灌入：`state.replace(&content, window, cx)`（打开时一次性读文件，
  同步读；v1 文件规模可控，未来大文件走后台线程——与 filetree 扫描同款）
- 只读：`Editor::readonly(true)`（渲染侧）+ `set_readonly`（状态侧）双保险
- 高亮：Cargo.toml `gpui-kit` 加 `syntect` feature；`gpui_component::init`
  已装 language provider，`.language("rust")` 即触发；**不启用 = 黑白**，
  这是 feature 存在的意义
- **备选（否）**：自研只读渲染（vt100 式行渲染）——重复造轮子，丢掉
  选区/复制/滚动/IME；tree-sitter——重量级，折叠等结构能力 v1 不需要

### D5: dedup 生命周期挂在 `Panel::on_removed`

`EditorPanel::on_removed` → weak app → `open_tab_removed(panel_id)` 从
dedup 表摘除。库保证签被关（含拖出/组移除）时调用。**不开新监听**：
`HashMap` 摘除后重开自然新建，场景 "Reopen after close" 成立。

### D6: 焦点模型 —— 三个落点

1. **键盘跟随 active tab**：`TerminalPanel::handle_key_down` 已有
   `focus.is_focused` 守卫——非 console 签 active 时焦点不在它身上，
   天然不转发（spec delta 定案的行为，实现近乎免费）
2. **sidebar 选/建 agent → `select_panel(console)`**：`on_sidebar_event`
   的 `SelectAgent`/`NewAgent`/`CloseAgent` 分支补一步；库
   `TabGroup::select_tab → focus_active_panel` 内建一步聚焦
   （TerminalPanel `Focusable` 已实现）
3. **树点击焦点交回**：tree 根级 on_click 冒泡从"focus 终端面板"改为
   "focus 当前 active tab 的面板"——console active 时行为与现状逐字相同
   （保住现有测试），viewer active 时归 viewer

### D7: console 签补 tab 标题

`TerminalPanel` 未实现 `title()` → 多签时 tab bar 显示默认 `Dock.Unnamed`。
实现 `title()` 返回固定 "console"（或活动 session 名——v1 取固定值，
避免标题随 session 切换抖动；扩展点注释标明）。

## Risks / Trade-offs

- **[file-tree delta 无法挂载直到前置归档]** → 依赖顺序：先归档
  `right-dock-file-tree`（17/18），再归档本 change；validate INFO 已确认
  此为唯一阻塞，实现不受影响（钩子签名在两 change 中兼容）
- **[tab 拖拽可把 viewer 拖到 console 前]** → v1 接受（D3）；spec 只写
  "console 是第一个页签"的创建序语义，不写"永远在第 0 位"；要钉死再加约束
- **[syntect 首次高亮延迟（LazyLock 反序列化 tens of ms）]** → 发生在
  首次打开签的渲染帧，可感知一次性卡顿；缓解：库已做 once 共享 + 64KB
  内同步解析、超限后台；可接受
- **[同步读文件阻塞 UI]** → v1 文件规模可控；风险登记：大文件打开卡顿
  → 后台读 + loading 占位（沿用 `starting_placeholder` 模式）留待后续
- **[`EditorState::new` 需要 `&mut Window`]** → open 链路（tree on_click）
  手里有 window，接口签名把 window 传到底即可；无窗口的调用方（未来
  命令面板）需另想办法——不在 v1 范围
- **[现有回归被焦点改动波及]** → `tree_click_keeps_keyboard_with_session`
  等测试的断言路径 = console active 场景，D6-3 保证行为不变；新场景
  （viewer active）写新测试，不动旧断言

## Migration Plan

单 PR 内完成，无数据迁移、无持久化格式变化。落地顺序：
1. Cargo.toml feature + `src/property_detail_manager/` 骨架（OpenObject/注册表/dedup/接缝）
2. `EditorPanel` + handler 注册 → 手动点 .rs 验证开签
3. 右钩子接入 + 焦点三落点（D6）+ console 标题（D7）
4. spec delta 对应的无头测试补齐，全量回归
回滚 = revert PR，无残留状态。

## Open Questions

（无——焦点模型、只读范围、handler 认领范围、高亮后端均已在探索中定案，
见 proposal.md What Changes。）
