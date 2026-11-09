# 设计：界面模块拆分 + Dock/Shell 机制（gpui-component DockArea 版）

> 状态：**初步设计，待对齐**
> 关联：`Docs/zed-ui-composition-research.md`（Zed 调研）、`openspec/specs/{agent-shell,workspace-management}`（行为约束）
> 性质：结构重构 + Dock 机制接入。**行为保持**（spec 不变），新增可见功能：右 dock（渲染/调整大小/关闭）
>
> **变更记录**
> - 2026-11-08：**D1 翻转** —— 由"自研轻量 Dock"改为**直接采用 gpui-component 的
>   `DockArea`**（用户决策：后续省事——tab 化/拖拽换位/持久化/resize/开合全部内建）。
>   本文档已按此全面修订；被推翻的旧结论见 §12 "已作废"。

---

## 0. 需求确认（来自讨论，已逐条确认）

| # | 需求 | 结论 |
|---|---|---|
| R1 | 整体按调研的短期方案，每个界面收为一个 Struct/对象 | ✅ 确认（落地方式因 D1 修订：面板为 **Entity**，见 D2） |
| R2 | 左边就是现在的 sidebar，**需要可以调整大小** | ✅ 确认（DockArea 内建 resize） |
| R3 | "外壳空间" = 窗口根布局容器（未来可挂 titlebar/状态栏/覆盖层） | ✅ 确认（DockArea 承担左右中布局；外壳退为 `TerminalApp::render` 外层容器） |
| R4 | 右 dock：**要渲染、可调整大小、可关闭**，作为技术验证，下一个功能模块就要用 | ✅ 确认（内建 toggle_dock + resize；占位面板 = 下个功能的接入样板） |
| R5 | 中间主面板收为一个 PaneGroup 对象 | ✅ 确认（**落地方式修订**：不再自研 PaneGroup，由 `DockArea` 的 center `DockLayout`/`PaneTree` 承担；我们的"主面板对象" = `TerminalPanel` 实体注册进 center。未来分屏 = `DockLayout::h_split()` 内建） |

---

## 1. 决策记录

### D1（2026-11-08 翻转）：采用 `gpui-component` 的 `DockArea`，不自研

**发现**：`gpui-kit 0.9` 经 `pub use ::gpui_component as component;`（gpui-kit
`src/lib.rs:170`）暴露完整 dock 系统：`gpui_kit::component::dock`（外观层）+
底层 `gpui-base 0.9` `src/dock/`（布局树/状态机/拖拽几何）。`gpui_kit::init`
已自动初始化其 `PanelRegistry`（gpui-kit `src/lib.rs:205-209`），无需额外 init。

**内建能力（自研版要写的代码，全部免费）**：

| 能力 | 内建实现 |
|---|---|
| 三槽布局 | `DockArea::render` = `size_full flex_row`：左 \| 中 \| 右 |
| 拖拽 resize | `resize_handle(...).on_drag(ResizePanel)` + `DockResizeTracker`（窗口级 mouse 流）+ `DockPlacement::clamp`（min 100px，max = 窗宽 − 100 − 对侧 dock） |
| 关闭/重开 | `toggle_dock(placement)`；`Dock { open, collapsible }` 默认 `true` |
| 尺寸播种 | `add_panel(..., size: Some(px(240)))`；缺省 `PANEL_MIN_SIZE * 2 = 200px` |
| 布局持久化 | `DockArea::new(id, version, ...)` + `load`/`dump → DockAreaState`（serde） |
| 未来 tab 化/拖拽换位/多面板同槽 | `DockLayout::tabs()` / `PaneTree` / `PanelView` 全套 |
| 未来分屏 | `DockLayout::h_split()/v_split().child(layout, size)` |
| 无头测试 | dock 模块自带 `test_support` + `DockArea::render` 内 `.test_support()` |

**代价（本轮必须消化）**：

1. **面板强制 Entity 化**：`Panel: EventEmitter<PanelEvent> + Render + Focusable`
   （gpui-base `dock/panel.rs:21`），plain struct 方案（旧 D2）作废。
2. **IME / Tab 键 / 一步切焦点 / 失效链**要随面板迁移重验 → §10 spike。
3. DockSkin 外观与现状有像素级差异（divider/border/padding）→ 接受为视觉变化，
   但不得破坏 spec 位置语义（见 D8）。

**升级/回退**：若 spike 发现 IME 或 Tab 路由在 DockArea 嵌套下无法保住 spec，
回退方案是仅用 `DockArea` 承担左右 dock、中心不动——该退路保留但预期不用。

### D2（修订）：面板 Entity 化，TerminalApp 仍是唯一 model/组合根

- `SidebarPanel` / `TerminalPanel` / `RightPlaceholderPanel` 均为 **entity**
  （`Panel` trait 强制），各自持有 `WeakEntity<TerminalApp>` 反向引用
  （**无强环**：TerminalApp → DockArea → 面板 →(weak)→ TerminalApp）。
- `TerminalApp` 保留：model（workspaces/active/save_error）、事件漏斗、
  输入/IME 桥、装配。**不引入**额外 observe/notify 仪式——gpui 的
  访问追踪失效链足够（§7）。
- 中间迁移期技巧：`Panel: Render`，面板实体可以**先作为普通 child 挂进
  现有布局**（`.child(panel.clone())`），步骤 2/3 无需 DockArea 也能编译出图。

### D3：ID 化 + 事件漏斗本轮做（不变）

- `active: Option<(usize, usize)>` → `Option<EntityId>`，事件载荷永不携带 idx，
  消除 listener 闭包捕获过期索引的隐患。
- `SidebarEvent` 枚举漏斗收拢到 `TerminalApp::on_sidebar_event` 单一入口
  （回调式不变，design D7 理由保留；为命令面板/快捷键复用铺路）。
  面板化后漏斗更自然：`SidebarPanel` 的 listener → weak app update → 漏斗。

### D4：Session ID 直接用 gpui `EntityId`（不变）

- `active: Option<EntityId>`，entity 创建即得，零新类型。Zed 同款先例
  （`Pane::preview_item_id: Option<EntityId>`）。
- **`agent_entities/` 因此零改动**（旧版曾计划挂 id 字段——不需要了）。
- workspace 身份 = `PathBuf`（spec 已定 unique identity）。显示名 `pi-N`
  （spec 的 per-workspace 计数器）与身份 id 相互独立，id 永不显示。

### D5（修订）：右 dock 开合路径 + 左 dock 可达性

| 操作 | 实现 |
|---|---|
| 关右 dock | 占位面板 header `×` → `TerminalApp::close_right_dock` → `toggle_dock(Right)`（**只切 open，不移除面板**），**关闭后焦点显式交回终端面板**（spike E2：点 × 会把焦点交给即将隐藏的面板） |
| 重开右 dock | **已取消（grill 定案 2026-11-08）**：本轮无会话内重开路径，重启恢复默认；占位面板 root `on_click` 也把焦点交回终端（占位不抢键盘，spec）。键位/内建按钮留待下个功能一并设计 |
| 左 dock | `set_dock_collapsible(Left, false)`：sidebar 是 spec 固定家具（`+ new workspace` 必须可达），不可关；resize 不受影响 |

### D6（修订）：持久化本轮仍不接，但成本已降至"调用一次"

`DockArea::new("main", Some(1), ...)` 的 `version` + `dump()/load()` 就绪；
本轮不调用（重启回默认）。触发信号：用户抱怨重启后布局丢失 → 加两行。

### D7（修订）：尺寸与夹取全部交给内建

- 播种：`add_panel(pane, placement, Some(px(240.)), ...)`（左 = 现 sidebar 宽）。
- 夹取：内建 `clamp`（min 100px；max = 窗宽 − 100px − 对侧 dock）。
- 旧版 `MIN_DOCK_SIZE=160 / MIN_CENTER_WIDTH=200` 等自定义常数**作废**；
  若视觉上 100px 下限太小，用 `PANEL_MIN_SIZE` 语义外的自定义 clamp 作为 Q4 备选。

### D8（新）：面板 chrome 策略 —— 全面 `title_bar() -> false`

gpui-component `Panel::title_bar()` 默认 `true`，注释明确 "false for a panel
that carries its own chrome"（`dock/panel.rs:144-148`）。我们的两个既有面板
都自带 chrome：

- **SidebarPanel**：`title_bar() -> false` —— 否则 DockSkin 标题栏压在
  `+ new workspace` 上方，直接违反 workspace-management spec（"button at the
  top of the sidebar"）。
- **TerminalPanel**：`title_bar() -> false` —— spec 的状态条
  （`● path — name [Status]`）在面板内部，不加第二层标题。
- 附带：`closable() -> false`（两个面板都不可从 tab 关闭）、
  `zoomable() -> false`（本轮不用 zoom）。
- 另设 `inner_padding()` 待 spike 确认（DockSkin 是否注入 padding）。

代价：内建 toggle 按钮（在 title bar 里）不可见 → 重开只靠 D5 键位（Q4 备选：
center 保留 title bar 换内置按钮）。

---

## 2. 目标模块结构

```text
src/
  main.rs ............. 入口 (TerminalApp::new 签名改为 (window, cx); 两处调用点均有 window)
  app.rs .............. TerminalApp = model + 组合根/外壳容器 + 事件漏斗 + IME/输入桥
  shell/
    mod.rs ............ 装配: DockSkin::dock_area 构建 + 三面板注册 + chrome 配置 (无状态函数)
    placeholder.rs .... RightPlaceholderPanel (impl Panel; 下个功能整体替换此文件)
  sidebar/
    mod.rs ............ SidebarPanel entity (impl Panel) + SidebarEvent + 行快照构建
  pane/
    mod.rs ............ TerminalPanel entity (impl Panel) + 网格渲染 + 焦点/键/IME 桥
                        (原 ui/pane.rs 全部自由函数 + 单测随迁)
  ui/
    mod.rs ............ 颜色映射 (theme, 保留)
    widgets.rs ........ TextTooltip 等共享小件 (可选, 最后一步)
  agent_entities/ ...... 不动 (D4: EntityId 即身份, 零改动)
  workspace/ ........... 不动
  tests/embed_layout.rs 不动 (不依赖 ui)
```

对比旧版：`shell/dock.rs`、`shell/pane_group.rs` **删除**（DockArea 取代）；
`Sidebar`/`TerminalPane` plain struct **升级为 entity**（D2）。

---

## 3. 状态归属

```text
TerminalApp (窗口根 entity; EntityInputHandler 留此)
  |-- workspaces: Vec<Workspace>       model: 路径/折叠/agents/显示名计数器
  |-- active: Option<EntityId>         导航状态 (D4)
  |-- save_error, grid_resizes, _update_task, _dialog_task
  |-- dock_area: Entity<DockArea>      布局引擎 (D1)
  |-- _dock_skin: Rc<DockSkin>         外观句柄 (chrome 配置入口, D8)
  |-- terminal: Entity<TerminalPanel>  一步切焦点用 (强引用; 面板→app 只持 weak, 无环)
  |
  `-- DockArea (entity, 内建)
        |-- Left  dock  ──> SidebarPanel      { app: WeakEntity<TerminalApp>, focus }
        |-- Center tree  ──> TerminalPanel    { app: WeakEntity<TerminalApp>, focus }
        `-- Right dock   ──> RightPlaceholderPanel { app: WeakEntity<TerminalApp> }
```

---

## 4. 渲染与装配

### 4.1 渲染

```text
TerminalApp::render (外壳空间, R3)
  div().size_full().flex().flex_col()
    .on_action(ToggleRightDock)                 <- 全局 action 挂根
    .child(self.dock_area.clone())              <- Entity<DockArea> 即整窗布局
    .child(<未来: titlebar / 状态栏 / 覆盖层>)     <- 外壳预留位, 本轮空
         |
         v
DockArea::render (内建): size_full flex_row
  [Left dock | Center | Right dock]   分隔线/边框/resize handle 均内建
  关闭的左/右 dock = 零像素 (render_dock: extent<=0 时不画)
         |
         v
各面板 render (我们写):
  SidebarPanel::render          读 app (workspaces/sessions) -> 行快照 -> 现 sidebar 全部内容
  TerminalPanel::render         读 app (active session)      -> #terminal-pane 整块
                                (key_context "Terminal" / track_focus / listeners / #terminal-grid / IME canvas)
  RightPlaceholderPanel::render header(标题 + ×) + body 提示   <- 下个功能的 chrome 样板
```

### 4.2 装配（`shell/mod.rs` 草图）

```rust
pub struct ShellParts {
    pub dock_area: Entity<DockArea>,
    pub skin: Rc<DockSkin>,
    pub terminal: Entity<TerminalPanel>,     // 供 TerminalApp 保存 (焦点)
}

/// 在 TerminalApp::new(window, cx) 内调用; weak_self = cx.weak_entity()
pub fn build(
    weak_app: WeakEntity<TerminalApp>,
    window: &mut Window,
    cx: &mut Context<TerminalApp>,
) -> ShellParts {
    let (dock_area, skin) = DockSkin::dock_area("main", Some(1), window, cx);
    // Some(1) = 持久化 schema 版本 (D6: 本轮不 load/dump, 先占位)

    let sidebar  = cx.new(|cx| SidebarPanel::new(weak_app.clone(), cx));
    let terminal = cx.new(|cx| TerminalPanel::new(weak_app.clone(), cx));
    let right    = cx.new(|cx| RightPlaceholderPanel::new(weak_app.clone(), cx));

    dock_area.update(cx, |area, cx| {
        // 必须 panel_handle 包装 (spike §10.5): 裸 entity 绕过 title_bar 判断,
        // 每槽多画 30px 标题栏; 顺序: 先 add (按需建 dock) 再 set_collapsible
        area.add_panel_view(panel_handle(sidebar), DockPlacement::Left,  Some(px(240.)), window, cx);
        area.add_panel_view(panel_handle(right),   DockPlacement::Right, Some(px(240.)), window, cx);
        area.set_dock_collapsible(DockPlacement::Left, false, window, cx);   // D5
        area.set_center(DockLayout::tabs().panel_view(panel_handle(terminal), cx), window, cx); // R5
    });
    // chrome 配置 (D8): 各面板 title_bar()/closable()/zoomable() 在 impl 内返回 false
    ShellParts { dock_area, skin, terminal }
}
```

要点：
- **注册必须 `panel_handle` 包装**（`add_panel_view` / `panel_view`）：
  裸 entity 注册时 `PanelHandle::of` 识别失败，`title_bar()=false` 被跳过 →
  每槽 30px 标题栏压住 `+ new workspace`（spike §10.5 实测 + gpui-component
  `panel.rs:320` 官方注释）。
- **顺序**：left/right 先 `add_panel_view`（顺带播种 240px 并按需建 dock），
  建完才能 `set_dock_collapsible`（对不存在的 dock 是 no-op）；center 用
  `set_center(DockLayout::tabs().panel_view(...))`。
- 面板实体即使暂不进 DockArea 也能独立渲染（`Panel: Render`）→ 迁移步骤可增量。

---

## 5. 各面板设计

### 5.1 `SidebarPanel`（`sidebar/mod.rs`，左 dock）

```rust
pub struct SidebarPanel {
    app: WeakEntity<TerminalApp>,
    focus: FocusHandle,               // Focusable trait 要求 (sidebar 不主动聚焦)
}

impl Panel for SidebarPanel {         // gpui-component Panel (含 base 半)
    fn panel_name(&self) -> &'static str { "sidebar" }   // 持久化稳定 id
    fn title_bar(&self, _: &App) -> bool { false }       // D8: spec 要求 + new workspace 置顶
    fn closable(&self, _: &App) -> bool { false }        // spec 可达性
    fn zoomable(&self, _: &App) -> bool { false }
    fn title(&mut self, ...) -> impl IntoElement { "workspaces" }  // title_bar=false 时仅兜底
}
impl EventEmitter<PanelEvent> for SidebarPanel {}
impl Focusable for SidebarPanel { ... }

impl SidebarPanel {
    fn render(&mut self, window, cx) -> impl IntoElement {
        // 读 self.app -> 构建 WorkspaceRow/AgentRow 快照 (拥有值, 现逻辑)
        // 现 ui/sidebar.rs 的全部行构建 + listener:
        //   cx.listener -> this.app.update(cx, |app, cx| app.on_sidebar_event(ev, window, cx))
    }
}

pub enum SidebarEvent {              // 漏斗载荷, 永不携带 idx (D3)
    NewWorkspace,
    ToggleCollapse(PathBuf), CloseWorkspace(PathBuf),
    NewAgent(PathBuf),
    SelectAgent(EntityId),            // handler: active = id + 焦点交 terminal 面板
    CloseAgent(EntityId),
}
// TextTooltip / WorkspaceRow / AgentRow 随迁
```

### 5.2 `TerminalPanel`（`pane/mod.rs`，center）

```rust
pub struct TerminalPanel {
    app: WeakEntity<TerminalApp>,
    focus: FocusHandle,               // 原 TerminalApp::pane_focus 迁入; 一步切焦点目标
}

impl Panel for TerminalPanel {
    fn panel_name(&self) -> &'static str { "terminal" }
    fn title_bar(&self, _: &App) -> bool { false }       // D8: spec 状态条在面板内
    fn closable(&self, _: &App) -> bool { false }        // 中心不可关
    fn zoomable(&self, _: &App) -> bool { false }
}
impl EventEmitter<PanelEvent> for TerminalPanel {}
impl Focusable for TerminalPanel { fn focus_handle(...) { self.focus.clone() } }

impl TerminalPanel {
    fn render(&mut self, window, cx) -> impl IntoElement {
        // 现 TerminalApp::render 中 "#terminal-pane" 整块原样迁入:
        //   .key_context("Terminal") .track_focus(&self.focus)
        //   .on_key_down   -> 键位映射(纯函数, 随迁) -> app.update -> app.send_key
        //   .on_action(TerminalTab) -> app.send_key(Tab)
        //   .on_click      -> self.focus.focus(window, cx)
        //   child: status_bar / separator / starting_placeholder / #terminal-grid
        //   canvas paint  -> self.app.upgrade() -> ElementInputHandler::new(bounds, app_entity)
        //                 -> grid_metrics -> clamp -> defer app.propagate_grid_size
    }
}
// 原 ui/pane.rs 自由函数随迁: render_empty / status_bar / terminal_grid /
//   grid_metrics / build_grid / PaneData / MIN_GRID_COLS / MIN_GRID_ROWS + 单测
```

**焦点检查迁移**：原 `handle_key_down` 的 `pane_focus.is_focused(window)` ——
listener 移到面板后改为检查 `self.focus.is_focused(window)`（同一 handle 被
`track_focus` 挂在 `#terminal-pane` 上，语义等价）。`send_key`/`send_text`/
`propagate_grid_size` 留在 `TerminalApp`（model 操作），面板经 weak 调用。

### 5.3 `RightPlaceholderPanel`（`shell/placeholder.rs`，右 dock）

```rust
impl Panel for RightPlaceholderPanel {
    fn panel_name(&self) -> &'static str { "right-placeholder" }
    fn title_bar(&self, _: &App) -> bool { false }       // 自带 header chrome
    fn closable(&self, _: &App) -> bool { false }        // 关的是 dock 不是 panel
    fn zoomable(&self, _: &App) -> bool { false }
}
// render: header("right dock" + × -> toggle_dock(Right)) + body(居中提示)
// 下个功能 = 整体替换此文件; dock 机制 (resize/open/panel 注册) 零改动
```

### 5.4 `TerminalApp`（瘦身后）

```rust
pub struct TerminalApp {
    workspaces: Vec<Workspace>,
    active: Option<EntityId>,
    save_error: Option<String>,
    grid_resizes: usize,
    _update_task: Option<Task<()>>,
    _dialog_task: Option<Task<()>>,
    dock_area: Entity<DockArea>,
    _dock_skin: Rc<DockSkin>,
    terminal: Entity<TerminalPanel>,
}

impl TerminalApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Result<Self, String>;
    //   bind_keys(TerminalTab) + store::load + shell::build(...)
    pub fn on_sidebar_event(&mut self, ev: SidebarEvent, window, cx);  // 漏斗
    // model 操作: create_agent / close_agent / toggle_collapse /
    //   close_workspace / create_workspace / persist
    // 输入: send_key / send_text / propagate_grid_size (面板经 weak 调)
    // ToggleRightDock -> dock_area.toggle_dock(Right, ...)
}
// EntityInputHandler impl 原样留此 (trait 约束); 键位映射 keystroke_to_keyevents 随迁 pane/
```

---

## 6. Dock 机制：需求 → DockArea 能力映射（R2/R4 对照表）

| 需求 | DockArea 内建 | 我们要做 |
|---|---|---|
| R2 左 dock = sidebar + 可调大小 | `add_panel(Left, Some(240))` + resize handle（on_drag + 窗口级 tracker + clamp） | 面板内容 + `title_bar false` + `set_dock_collapsible(Left, false)` |
| R4-a 右 dock 渲染 | `add_panel(Right, Some(240))`，默认 `open=true` | 占位面板（header × + body 提示） |
| R4-b 右 dock 可调大小 | 同上内建 | 无 |
| R4-c 右 dock 可关闭 | `toggle_dock(Right)`（只切 open，面板保留） | header `×` 调用 + `ToggleRightDock` 键位重开 |
| R4-d 技术验证/下个功能 | 面板注册管线 + `panel_name` 持久化 id | 占位面板即接入样板 |
| R3 外壳 | DockArea 自身 = 左中右行布局 | `TerminalApp::render` 外层容器（未来 chrome 挂载位） |
| R5 主面板对象 | center `DockLayout::tabs()` + `PaneTree`；未来 `h_split()` 分屏 | `TerminalPanel` 实体 |
| R1 模块收对象 | `Panel` trait 强制 Entity 化 | 各面板 + 漏斗（D3） |
| 尺寸夹取 | min 100px / max = 窗宽−100−对侧（`DockPlacement::clamp`） | 无（Q4 备选：自定义下限） |
| 布局持久化（D6 未来） | `version` + `load`/`dump` | 调用两行 |

### 关闭语义（易错点，写明）

```text
toggle_dock(Right)   = dock.is_open = false   -> 零像素, 面板/尺寸/状态全保留
ClosePanel action    = 从 tab 组移除 panel     -> dock 变空被 normalize 掉, 重开要重新 add
                       ^ 我们不用这个 (占位面板 closable=false 也挡住它)
```

---

## 7. 事件流与失效链（设计不变式）

### 7.1 事件流

```text
SidebarPanel::render
  └─ cx.listener -> this.app.update(cx, |app, cx| app.on_sidebar_event(ev, window, cx))
        |-- NewWorkspace / ToggleCollapse(path) / CloseWorkspace(path)
        |-- NewAgent(path)
        |-- SelectAgent(id)  -> active = id + terminal.focus.focus(window, cx)   (一步切焦点 spec)
        `-- CloseAgent(id)   -> kill + 移除 + active 补偿 (关 active -> 同 ws 最后一个 / 无则 None)

TerminalPanel::render
  ├─ on_key_down    -> keystroke 映射 -> app.send_key   (focus 检查用面板自己的 handle)
  ├─ on_action Tab  -> app.send_key(Tab)                (Terminal context 绑定, D5 路由)
  ├─ on_click       -> self.focus.focus()
  └─ canvas paint   -> app.propagate_grid_size (defer; 与现路径同)

RightPlaceholderPanel header × / ToggleRightDock action
  -> dock_area.toggle_dock(Right, window, cx)
```

### 7.2 失效链（gpui 访问追踪；**面板 render 必须穿透读到 session**）

> **⚠ 2026-11-08 实测修正（见 §10.5）**：本节“穿透读即自动刷新”只对根视图
> 成立——嵌套面板不因自读而重建。落地时必须补充 **notify 转发**：数据变化 →
> 显式 notify 展示该数据的面板实体（drain 循环 / model 漏斗中转发）。

```text
gpui 机制: 窗口渲染期间被 read 的 entity -> 记入 tracked_entities
           该 entity notify() -> 窗口重绘 -> 全树 render 重新执行

poll 循环: session.update(|s,cx| if s.0.poll() { cx.notify() })
           -> TerminalPanel::render 读过 session (screen/status) -> 窗口重绘
           -> SidebarPanel::render 同帧重读 (badge 更新)           ✓

TerminalApp 方法 cx.notify() (如 create_agent)
           -> SidebarPanel::render 读过 app (workspaces) -> 窗口重绘  ✓
```

**不变式**：
1. 面板 render **每帧现读**（行快照/PaneData 在 render 内构建），不得把跨帧数据
   缓存进面板字段——否则 notify 到了但 render 读到旧值。
2. 谁展示谁读：sidebar 读 sessions（badge），terminal 读 active session（grid）。
   窗口级失效任一读取即触发全树重绘，但各自必须读到自己展示的数据。

---

## 8. 迁移步骤（每步可编译、`cargo test` 全绿）

| 步 | 内容 | 验收 |
|---|---|---|
| 0 | **门禁 spike**（§10，临时分支，验证后丢弃）：焦点 containment T1-T6/I1-I2 + 顺带 V1-V7 | 全部通过才进 step 1；失败则停迁移，走 §10.4 预案 |
| 1 | **ID 化**：`active: Option<EntityId>`，事件载荷去 idx（D3/D4） | `cargo test` + 增删切换冒烟 |
| 2 | **SidebarPanel 实体化**：`ui/sidebar.rs` → `sidebar/mod.rs`，impl Panel，listener 改 weak app + 漏斗；**先作为普通 child 挂进现布局**（旧左槽位置） | 点击行为全通；spec 按钮可用 |
| 3 | **TerminalPanel 实体化**：`ui/pane.rs` → `pane/mod.rs`，focus/键/IME 桥迁入，`#terminal-pane` 整块移入；同样先挂现布局 | `adaptive_grid_dispatch` 绿（`find("terminal-grid")`）；中文 IME 可输入 |
| 4 | **装配切换**：`TerminalApp::new(window, cx)` + `shell::build`；render 换成外壳 + `dock_area`；右占位面板注册；chrome 配置（D8/D5） | §9.2 清单逐条过 |
| 5 | (可选) `ui/widgets.rs` 拆分，TextTooltip 归位 | `cargo test` |

顺序理由：0 先排掉机制风险；2/3 借"实体可独立渲染"（D2）保持视觉不变、
测试不动；4 是纯装配切换（一次 render 替换）；焦点查询路径在 4 中只改一处
（`self.pane_focus` → `self.terminal` 面板 handle）。

**签名变更**：`TerminalApp::new(cx)` → `new(window, cx)`。两处调用点
（`main.rs` 的 `open_window` 回调、`adaptive_grid_dispatch` 测试）都已在
`|_window, cx|` 作用域内有 window，改动为直传。

---

## 9. 测试影响与验收清单

### 9.1 测试影响

| 测试 | 影响 | 处理 |
|---|---|---|
| `app.rs::adaptive_grid_dispatch` | ① `TerminalApp::new` 签名 ② `app.active = Some((0, idx))` ③ `MIN_GRID_COLS/ROWS`、`grid_metrics` 导入路径 ④ **新增：无头窗口 + DockArea 渲染** | ① 直传 `_window` ② `Some(entity.entity_id())`（先取 id 再 move）③ `crate::pane::{...}` ④ spike V6 覆盖（dock 自带 `test_support`，预期可行） |
| 断言兼容性 | `bounds < 800`（1000px 窗）：左 240 + 右 240（默认开）+ chrome → 中心 ≈500 ✓；右关后 ≈750 ✓。窗口 resize 后中心变宽 ✓（dock 尺寸固定，增量全给中心） | 步骤 4 后跑一遍确认；若 DockSkin padding 使断言越界，调窗口尺寸而非断言 |
| `ui/pane.rs` 网格单测 | 随模块搬 | 导入路径 → `crate::pane::grid_metrics` |
| `tests/embed_layout.rs` | 无 | 不动 |

`window.find("terminal-grid")`、`.test_support()` 钩子、`grid_resizes`
诊断语义**不得改变**（spec/回归依赖）。

### 9.2 功能验收清单（对照 R1–R5）

- [ ] **R1 结构**：三个面板各为一个 entity（impl Panel）；`app.rs` 无整块
      渲染代码；`TerminalApp` = model + 外壳容器 + 漏斗 + 输入桥
- [ ] **R2 左 dock**：sidebar 内容/spec 行为逐项不变；右缘拖拽可调宽；
      松手后网格随 bounds 重算列行；左 dock **无关闭入口**（D5，spec 可达性）
- [ ] **R3 外壳**：`TerminalApp::render` 是唯一窗口根容器（外壳），
      DockArea 是其子；未来 chrome 有明确挂载位
- [ ] **R4 右 dock**：
  - [ ] 默认渲染（占位 chrome：header + × + body 提示）
  - [ ] 左缘拖拽可调宽度（内建 clamp）
  - [ ] `×` 关闭 → 零渲染、中心变宽、网格重算；**面板状态保留**
  - [ ] `ToggleRightDock` 键位重开 → 恢复原存储宽度
  - [ ] 关闭/重开/拖拽不产生 console 报错、不丢焦点（终端仍可输入）
  - [ ] （占位即样板）下个功能只需替换 `shell/placeholder.rs` 并重注册
- [ ] **R5 主面板**：中心经 `DockLayout::tabs().panel(terminal)` 出图；
      焦点查询唯一入口 = `TerminalPanel::focus`
- [ ] **spec 回归**：`cargo test` 全绿；`+ new workspace` 是 sidebar 顶部
      第一个可交互元素（D8）；一步切焦点；Ctrl 不转发；中文 IME 可输入
      （commit 上屏，preedit 不显示）；Tab 键仍转发进终端（D5 路由）
- [ ] **失效链**：agent 输出滚动实时更新（终端 + sidebar badge 同步）

### 9.3 OpenSpec 关联

- 结构拆分（R1/R3/R5）：**无 spec 影响**，不产生 delta。
- 右 dock（R4）+ 左 dock resize（R2）：新增可见能力，实现阶段应作为
  **delta 提案**（新 capability 如 `dock-layout`），propose 阶段定。

---

## 10. Spike（步骤 0）：IME/Tab 风险机制分析 + 门禁验证

> **形式（已确认）**：临时分支一次性验证，**验证后丢弃**，不留测试资产。
> **地位**：迁移门禁——全部通过才进入 step 1；任一失败则停止迁移，
> 先走 §10.4 预案探索出方案，再重启整体迁移。

### 10.1 机制分析：两条路由的现有原理

**Tab 路由（design D5）**：

```text
注册:  cx.bind_keys([KeyBinding::new("tab", TerminalTab, Some("Terminal"))])   app.rs:86
挂载:  #terminal-pane .key_context("Terminal") .track_focus(&pane_focus)        app.rs:555

按键: gpui 沿焦点路径收集 key_context -> context_stack = [Root, ..., "Terminal"]
      binding_enabled() 返回 binding context 在栈中的 depth        keymap.rs:202
      命中 binding 按 depth 降序 (同深按注册序) 取胜               keymap.rs:216
      -> "Terminal" 压过 Root 的焦点遍历 tab 绑定
      -> TerminalTab action 派发 (原始 key event 被吞 -> 走 on_action 非 on_key_down)
```

胜负只取决于 `context_stack` 是否含 `"Terminal"` 及其深度。

**IME 路由**：

```text
paint: canvas 闭包 window.handle_input(&pane_focus,
         ElementInputHandler::new(bounds, TerminalApp entity), cx)
       -> 按 focus handle 登记输入处理器
平台 IME 事件 -> window 查"当前聚焦 handle"的 handler
       -> TerminalApp::replace_text_in_range -> send_text -> pty
       bounds_for_range 返回 element bounds -> 候选框定位
```

IME 只认**当前聚焦的那个 handle**：焦点在我们 handle 上则一切正常，
焦点逃逸则 handler 查不到 → 中文输入静默死亡。

**收口：两者共享同一根因 = 焦点是否留在 TerminalPanel 的 focus handle 上**：

```text
                 焦点在 TerminalPanel 的 focus handle?
                    /                        \
                  是                          否
                 /                              \
  context_stack 含 "Terminal"            Tab: 落到 Root 遍历 (spec 违规)
  Tab -> TerminalTab action   OK          IME: 查无 handler (中文输入死) NG
  IME -> 我们的 handler        OK
```

所以 spike 不是验两件事，是**验一件事：焦点 containment**。

### 10.2 静态核查结果（已做完：递归 grep gpui-base + gpui-component 全部 dock 源码）

| 检查项 | 结果 |
|---|---|
| dock 注册 KeyBinding / bind_keys / keymap | **零** |
| dock 元素设置 key_context | **零** → context_stack 不增条目，`"Terminal"` 深度不变 |
| dock 挂 on_key_down | **零**（仅 `ToggleZoom`/`ClosePanel` 两个 on_action，不碰 Tab） |
| dock 注册 handle_input / InputHandler | **零** → 不可能覆盖我们的 IME handler |
| `gpui_component::init` 注入默认键位 | **否** |
| 焦点移动点全枚举 | ① `select_tab` → `focus_active_panel` → `Panel::focus_handle()`（= 我们的 handle，落到我们身上）② `remove_panel` handoff（我们不移除面板，不触发）③ 点击 tab（title_bar=false + 单面板 = 无 tab 可点）④ `toggle_dock` 源码确认只切 open + reconcile，**完全不动焦点** |

**结论**：dock 无任何劫持路径；其焦点 handoff 全部走 `Panel::focus_handle()`，
与我们设计中 pane 的 handle 是同一个——机制上是顺的。剩余风险只在
**边界情况的焦点逃逸**（下表 E1-E5）。

### 10.3 验证场景与通过标准

**风险场景（spike 逐一过）**：

| # | 场景 | 风险 |
|---|---|---|
| E1 | 点击 panel 内我们 div 覆盖不到的区域（dock inner padding / 边框 / resize handle 附近） | 焦点去哪？gpui 点非 focusable 区域是否保持焦点？**最大未知数** |
| E2 | 焦点在右 dock 面板里 → `toggle_dock` 关闭它 | 面板 offscreen 后焦点被 gpui 怎么处理？落到哪？能否回终端？ |
| E3 | resize 拖拽（mouse capture 全程）结束后 | 焦点是否还在原处 |
| E4 | sidebar 一步切焦点 → Tab/IME 即刻生效（spec 验收） | dock 嵌套下 focus containment 是否放行 |
| E5 | 真实中文 IME：候选框位置 / commit 上屏 / preedit 不显示 | 无头测不了（NSTextInputClient 平台侧），**必须 headed 手动** |

**通过标准（可判定）**：

```text
无头 (gpui_kit::test, 临时分支内自动跑):
  T1  focus pane -> dispatch "tab"   -> TerminalTab handler 触发, focus 未变
  T2  点击其他面板再回来 -> tab        -> 触发
  T3  toggle 右 dock 开/关 后 -> tab    -> 触发 (焦点已回/仍在 pane)
  T4  resize 拖拽后 -> tab             -> 触发
  T5  每步后断言 window.focused() == pane 的 handle   <- 焦点 containment 核心断言
  I1  focus pane -> 注册的 input handler 收到注入的 commit 文本

headed 手动 (一次, 约 5 分钟):
  I2  中文输入: 候选框出现/位置合理/commit 上屏/preedit 不显示
  T6  真实 Tab 键在中文输入过程中/之后仍转发
```

**门禁**：

```text
              spike (step 0, 临时分支)
                   |
        +----------+----------+
        |                     |
     全 T1-T6/I1-I2 通过    任一失败
        |                     |
   进入 step 1             停止, 不迁移
   (ID化->面板->装配)       走 §10.4 预案, 探索出方案后再重启迁移
```

**同分支顺带验证**（非 IME/Tab，但同批做完避免二次开分支）：

| # | 验证项 |
|---|---|
| V1 | `DockSkin::dock_area` + `add_panel`/`set_center` 在 gpui-kit 0.9 测试窗口跑通 |
| V2 | `title_bar() -> false` 后 sidebar 顶部 = `+ new workspace`（连带确认 `inner_padding()`） |
| V3 | 失效链：poll → `session.notify()` → 窗口重绘 → 两面板刷新（§7.2 不变式） |
| V4 | resize 拖拽：拖出面板区域仍跟踪、窗口变小 clamp、松手后网格重算 |
| V5 | 右 dock `toggle_dock` 关/开全程：零像素、恢复尺寸；内建 toggle 按钮在我们 chrome 下是否可见（决定 D5 键位是否唯一路径） |
| V6 | 无头 `window.find("terminal-grid")` / `debug_bounds` 可达（步骤 4 测试可行性） |
| V7 | 键位 `ctrl-shift-2` 全局触发（焦点在 sidebar/终端/空白时均有效） |

### 10.4 失败预案（预先定义，失败时不抓瞎）

| 失败类型 | 预案 | 评估 |
|---|---|---|
| **T5 焦点逃逸**（E1/E3 类） | **F1 焦点兜底**：panel `set_active(true)`（dock 切换时必回调）与 `on_click` 里显式 `focus.focus()`；必要时订阅 area 焦点丢失事件重定向 | 最可能的失败形态，代价小，几乎必然可解 |
| **T1-T4 Tab 不触发**（若 dock 注入 context） | **F2 绑定加固**：提高我们 binding 的优先级 meta，或用更具体 context 组合 | 当前源码零 context，概率很低 |
| **I1/I2 IME 死** | **F3 结构排查**：handler 按 handle 登记，焦点修好 IME 即好（同 F1）；若真是 `bounds_for_range` 被 dock 裁剪容器影响 → 定点修 | 与焦点强耦合，独立坏的概率低 |
| **焦点机制根本冲突**（兜底全部无效） | **F4 回退自研 Dock**（旧 D1 设计，见 §12 已作废记录） | 见下「关键保障」，损失可控 |

**关键保障：迁移步骤天然被门禁保护**：

```text
step 0  spike           <- 门禁, 在此拦截
step 1  ID 化            ┐
step 2  SidebarPanel     ├ 不依赖 DockArea: 面板以普通 child 挂进现有
step 3  TerminalPanel    ┘ 布局, 视觉/测试/IME/Tab 全是老路径
step 4  装配切换         <- 只有这一步依赖 spike 通过
```

即使 F4 回退，**steps 1-3 的模块拆分（R1 核心诉求）照常交付**——
回退只损失“内建 resize/toggle”，回到旧自研 Dock 设计。即：
**探索在最前，迁移被门禁，最坏回退不伤已交付部分。**

### 10.5 执行结果（分支 `spike/dockarea-gate` 实测，2026-11-08）

> 无头门禁 **T1-T5/I1 + V1-V6 全部通过**（`cargo test --test spike_dock_focus`，
> 基线 8+1 测试不受影响）。T6/I2（headed 真实中文 IME）待手动执行。
> spike 文件 `tests/spike_dock_focus.rs` 按计划验证后丢弃。

**关键实测发现（影响后续步骤写法）**：

1. **注册必须用 `panel_handle` 包装**（`add_panel_view` / `panel_view`）：
   裸 entity 注册时 `PanelHandle::of` 识别失败，`title_bar() = false` 被跳过，
   **每槽必画 30px 标题栏**，`+ new workspace` 被压到第二行。
   gpui-component `panel.rs:320` 官方注释同款结论。→ 步骤 5.1 的
   `shell::build` 必须写 `panel_handle(...)` 变体。
2. **失效链实测模型（修正 §7.2 原表述）**：
   - 非视图 entity notify → **只重建“读它的根视图”**（counter 轮：根 81→82，
     面板 77→77）
   - 嵌套面板**不因自读而重建**（only2 轮：77→77）
   - **直接 notify 面板实体 → 自动重绘 + 重建 ✓**（77→78）——设计采用此路
   - **设计约束**：数据变化必须显式 notify 展示该数据的面板（drain 循环 +
     model 漏斗转发 notify）；仅靠“面板穿透读 session”**不足以**保证刷新
3. **焦点 handoff**：点击 `×`（面板内控件）会把焦点交给即将隐藏的面板
   （E2 实测：关闭后焦点 = right(offscreen)）；**程序化 toggle 不动焦点**
   （T3：关闭后焦点 = terminal ✓）。→ 生产 `×` handler 关闭后需显式把焦点
   交回 terminal pane（F1 兜底的具体落点）；恢复路径（点终端）已验证 ✓
4. **其余确认**：
   - `title_bar=true` → 30px 标题栏（knob 生效）；生产 false/false 时三面板
     顶对齐 y=0、btn_dy=0（`+ new workspace` 真置顶）
   - `inner_padding` 单面板时不生效（`content_frame` 仅 `panels.len()>1` 才
     加 padding）→ D8 连带确认：无影响
   - `resize-handle-left` 可被 `find` → 拖拽 240→317.5px，clamp 路径通
   - Tab 绑定截获原始 key（`key_downs` 0→0，与 D5 同款）；绑定路由不受
     DockArea 嵌套影响（零 key context / 零 binding 静态核查得实证）
   - IME：`input("你好")` → 焦点 handle 的 input handler → EntityInputHandler
     收到完整 `"你好"`（I1 ✓；真实平台 NSTextInputClient 路径待 I2）
   - gpui-fast 测试模式在 effect 处理时自动 draw 脏窗口（app.rs:2176）：
     notify 面板后 `run_until_parked` 即重建，无需手动 render

**spike 工程备忘**：

- `#[gpui_kit::test]` 在集成测试中**不能** `use gpui_kit::*`——glob 会把
  生成代码里的 `#[test]` 遮蔽成 gpui 属性 → 无限递归/SIGBUS；必须显式导入
  （`src/app.rs` tests 模块同款注释）
- 外部实体变更后 `find` 读的是上一帧观测 → 需先 `render_frame`
- `on_click` 在 `StatefulInteractiveElement` 上：须在 `.id()` 后、
  `.test_support()` 前调用；supertrait 方法（`panel_name` 等）写在 `BasePanel` impl

**headed（1.2）**：~~I2 中文 IME 候选框/上屏/preedit、T6 真实 Tab 键~~
→ **已通过（2026-11-08 用户实测，示例 `examples/spike_dock_ime` 随分支丢弃）**。

### 10.6 装配阶段实现结论（步骤 5 实测，2026-11-08）

1. **门禁全过**：1.1–1.4 完成，分支已丢弃，master 基线不受影响。
2. **`panel_handle` 生产验证**：三槽布局 ✓（左 240 / 中 520 / 右 240）；
   `title_bar()=false` 生效（sidebar-root y=0，`+ new workspace` 置顶 ✓）。
3. **中心槽 pane 尺寸策略（新知识，二分实测）**：
   - **不能加 `.flex().flex_col()`**——gpui 对 flex 容器的子项按**内容拥抱
     宽度**：grid 内容宽 780px 直接撑爆 520px 中心槽（溢出到 1038，自适应
     量到错误宽度）；
   - 非 flex 子项**默认纵向堆叠**（status/separator/grid 层叠 ✓）且宽度填充
     槽位（520 ✓）——无需 flex_col；
   - 最终样式：`overflow_hidden + h_full + min_w/min_h(0)`（无 flex）→
     pane 精确 520×600，自适应双轴正常（64→90 列、24→28 行）；
   - grid 的 `min_w(0)` 注释同款问题在 pane 上复现：min-content 反馈链会把
     自动最小宽度撑到内容宽。
4. **notify 转发落地（§7.2 修正案）**：`notify_panels`（drain 循环 +
   sidebar 漏斗 + 对话框完成回调）→ 面板实体 notify → 自动重绘 ✓。
5. **焦点交回落地**：`close_right_dock`（toggle 后交回终端焦点）；占位面板
   root `on_click` 同样交回（占位不抢键盘，spec）。
6. **观测锚点**：`terminal-pane` / `sidebar-root` / `right-root` /
   `right-header` 加 `.test_support()`（非测试构建 no-op），供回归 `find`。
7. **签名落地**：`TerminalApp::new(window, cx)`（main.rs 与测试直传）。
8. **F2 光标空隙修复（6.1 手动验收发现，2026-11-09）**：中文输入后光标
   视觉后移、每字 +1 半角 cell。根因：gpui 对 CJK 字形按半角推进
   （真实渲染路径实测 7.8px/字，无全角归一），而 `build_grid` 同色合并段
   按 cell 数强制段宽（2×cell/字）→ 空隙 = 字数 × 7.8 堆积在段尾。
   **既有问题非本次回归**（build_grid/terminal_grid/grid_metrics 与 HEAD
   逐字 diff IDENTICAL）。修复：宽字符立即独立成段（每字独占 2-cell 盒 →
   字距 = 标准全角、文字段总宽 = 段宽、光标紧贴文字尾）；
   测试：`wide_chars_are_isolated_segments_f2` 锁定 + 续格用例改 2 段。
9. **F3 Tab 转发验证（6.1 验收，2026-11-09）**：应用侧全链证实——真机
   `[F3] TerminalTab action fired` ×3（绑定截获路径）+ CI `tab_forwards`
   计数断言（`sidebar_select_grants_one_step_focus` 断言3，spec "Tab SHALL
   be delivered" 守卫）。**pi 收到 Tab 零反应**（诊断 inject Tab：光标+整屏
   双零，同通道 Left/Right/Backspace 均有反应）→ pi-shell-rs 侧问题，
   已记 `Docs/pending-decisions.md` 待查（用户：原生 TUI 有此功能）。
10. **测试 hermetic 化（6.1 收尾发现，2026-11-09）**：手动冒烟清空
    `.workspaces.json` 后 4 个测试齐挂（"至少一个 workspace" 断言）——
    原 adaptive 测试本就依赖开发者本地文件状态。修复：`inject_test_workspace`
    向模型注入专用 in-memory workspace（不写盘），4 测试全部改为操作注入的
    workspace，不再依赖本地文件内容。
11. **重入 panic 修复（6.1 首跑发现，2026-11-08）**：sidebar listener 在
   **面板 lease 内**执行（gpui dispatch = `panel.update(闭包)`），funnel 的
   `notify_panels` 若同步 `panel.update` 即重入 →
   `cannot update SidebarPanel while it is already being updated` → abort。
   **修法**：`notify_panels` 改 `cx.defer`（`App::defer` 的 effect 只在最外层
   `finish_update` flush，此时面板 lease 已释放）；顺带补全 3.3 遗留——
   转发列表加 **terminal 面板**（输出滚动，spike §10.5 已证面板自读不重建）。
   **回归测试** `sidebar_row_click_toggles_collapse`：点击行 → funnel →
   defer → 面板重渲（折叠后 `[+ agent]` 从观测消失）；已反向验证——
   改回同步版测试复现完全相同的 panic。

---

## 11. 风险

| 风险 | 等级 | 对策 |
|---|---|---|
| 焦点逃逸 → Tab/IME 同时失效（E1-E5，同一根因） | **高** | §10 门禁 spike 优先验证；失败走 F1 焦点兜底；根因级冲突走 F4 |
| 失效链断裂输出不刷新（V3） | 中 | §7.2 不变式写进代码注释；spike V3 验证 |
| DockSkin 像素差异破坏测试断言 | 低 | 断言均为相对值（§9.1 已算过账）；只调窗口尺寸不改断言 |
| gpui-component dock 是 0.9 新模块，0.10 可能变 | 中 | 版本锁在 Cargo.toml（现有依赖策略一致）；对外只用 `component::dock` 再导出面 |
| 无头测试下 DockSkin 表现未知（V6） | 中 | dock 自带 `test_support` 是好信号；不行则该测试改"装配后 bounds 断言"的变体 |

---

## 12. 本轮不做 / 已作废

**本轮不做**：
- 多面板同槽 tab 化、拖拽换位、面板拆分（DockArea 内建但**不启用**——每槽单面板）
- `load`/`dump` 持久化接线（D6）、bottom dock、zoom（`ToggleZoom`）、`ClosePanel` 流程
- 状态栏 / titlebar / 覆盖层实现（外壳只留挂载位）
- 双击 reset 尺寸（内建也未提供则不加）

**已作废（旧版结论，防误读）**：
- ~~自研 `Dock`/`PaneGroup` struct~~ → DockArea 取代
- ~~面板 plain struct（旧 D2）~~ → `Panel` trait 强制 Entity（新 D2）
- ~~`MIN_DOCK_SIZE=160 / MIN_CENTER_WIDTH=200` 自定义夹取（旧 D7）~~ → 内建 clamp
- ~~`agent_entities` 挂 id 字段（旧 D4 附带）~~ → EntityId 零改动
- ~~调研"刻意不学 Panel trait"~~ → trait 现在是必选项；被保留的"不学"是
  250-crate 分层与自研布局树

---

## 13. 开放问题

| # | 问题 | 状态 / 默认提案 |
|---|---|---|
| Q1 | 右 dock 重开键位 | **已定案（grill 2026-11-08）**：不需要键位，本轮无会话内重开路径 |
| Q2 | 右 dock 默认宽度 | **已定案**：240px（`add_panel` size 播种，已实现） |
| Q3 | 左 dock 是否允许关闭 | **已定案**：不可关（`set_dock_collapsible(Left, false)`，已实现）；将来要收起 sidebar 再改 true 并补重开 UI |
| Q4 | chrome 策略 | **已定案（grill 2026-11-08）**：全 `title_bar() -> false`（已实现，生产验证 sidebar-root y=0） |
| Q5 | 内建 100px 最小宽是否够 | 保留观察：当前 dock 240px，min100 未被视觉检验；拖到极窄时再评估自定义 clamp |

---

## 附录 A：DockArea API 速查（gpui-kit 0.9 实测出处）

```text
路径: gpui_kit::component::dock   (= gpui-component::dock; 布局引擎在 gpui-base::dock)
初始化: gpui_kit::init 已含 (PanelRegistry 按需自建, 无需手动)

构建:
  DockSkin::dock_area(id, version: Option<usize>, window, cx) -> (Entity<DockArea>, Rc<DockSkin>)
  或 DockArea::new(id, version, window, cx).with_renderer(DockSkin::new(cx))

注册/布局:
  area.add_panel<P: Panel>(Entity<P>, DockPlacement, size: Option<Pixels>, window, cx)
  area.set_center(DockLayout, window, cx)
  area.set_dock(placement, DockLayout, window, cx)        // 含 center (转发 set_center)
  DockLayout::tabs().panel(entity).active_index(ix)
  DockLayout::h_split()/v_split().child(layout, Some(size))

开合/尺寸:
  area.toggle_dock(placement, window, cx)                 // 只切 open; 需 collapsible
  area.is_dock_open(placement) / dock_size(placement)
  area.set_dock_collapsible(placement, bool, ...)
  area.set_dock_size(placement, ...) / set_locked(...)
  clamp 内建: min PANEL_MIN_SIZE=100px; max=窗宽-100-对侧 (dock_placement.rs:92)

持久化 (D6 未来):
  area.dump(cx) -> DockAreaState (serde); area.load(state, window, cx)
  id/version 在 DockArea::new 持有; DockEvent::LayoutChanged / DockToggleEvent 可订阅

Panel trait (我们实现的面):
  base  (gpui_base::dock::Panel = BasePanel): EventEmitter<PanelEvent> + Render + Focusable
        必写: panel_name() -> &'static str (稳定持久化 id)
        默认即可: visible/closable/zoomable=true, set_active/set_zoomed/on_added_to...
  展示 (gpui_component::dock::Panel: BasePanel):
        title_bar() -> bool   (默认 true; false = 自带 chrome — D8 用它)
        title()/tab_name()/title_style()/title_suffix()/toolbar_buttons()/dropdown_menu()
  PanelEvent { ZoomIn, ZoomOut, LayoutChanged }
  PanelStyle::Auto: 单面板显示 simple title 而非 tab 栏 (skin.set_panel_style)

内建 chrome (DockSkin):
  resize handle: dock.rs:142 resize_handle(id, axis).inside(edge).on_drag(ResizePanel, ...)
                 + DockResizeTracker (窗口级 mouse 流, paint 阶段注册)
  toggle 按钮: tab_panel.rs:218 dock_toggle_button — 渲染在 center 面板 title bar
               (is_toggle_button_visible 默认 true; title_bar=false 时不可见)
  close 按钮: set_close_button_visible (默认 false)
  actions: dock::{ToggleZoom, ClosePanel}   (我们不用, 见 §6 关闭语义)
  test_support: dock 模块自带 (headless)
```
