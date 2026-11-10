# Design

## Context

- 右槽现状：`RightPlaceholderPanel` 是官方接入样板（注释承诺"替换本文件即接入，dock 机制零改动"）；`shell/mod.rs` 经 `panel_handle` 包装注册（裸 entity 会多出 30px 标题栏，spike §10.5）。
- UI 能力现成：gpui-component 0.9.0 自带 `Tree`（`TreeState`/`TreeItem`/`TreeEvent`，uniform-list 虚拟滚动）、逐行 `context_menu` 构建器、`PopupMenu`。库的左键内置行为 = 选中 + 展开切换；`TreeEvent` 只有 `Expanded/Collapsed`，**无点击事件**。
- 库的坑：`TreeItem.children` 是 eager `Vec`，`is_folder() = !children.is_empty()`（空目录长得像文件）；`set_items` 整树重建 = 全新 `Rc` 展开态 = 展开全丢。
- 上一轮（split-ui-modules-dockarea）确立的模式：面板实体持 `WeakEntity<TerminalApp>`、跨面板通知走 `cx.defer`（防重入 panic）、漏斗入口焦点交回终端。
- `.workspaces.json` 现为单一 `workspaces` 键；`store.rs::save(paths)` **按 paths 重建整个对象**——不改签名会把新区段洗掉。
- `agent-shell` 键盘契约绝对化（"仅 macOS 失焦不转发"）→ v1 右 dock **不得存在可聚焦文本输入**，过滤配置化正是绕开点（见 proposal Why）。
- 本设计动机见 proposal.md；行为契约见 specs/（file-tree 7 需求、dock-layout/workspace-management 修订）。

## Goals / Non-Goals

**Goals:**

- 右槽填入文件树面板，dock 机制（resize/close/注册）零改动
- 插件化预备：dyn-safe trait + 中立 DTO，面板从第一天持 `Box<dyn _>`
- 忠实 gitignore 过滤引擎（文件行限定），配置驱动、v1 零输入控件
- 扫描/刷新不阻塞 UI；展开态在刷新与过滤下均不丢失

**Non-Goals:**

- 默认打开系统（左键 open 钩子 v1 为 no-op，另行立项）
- 配置编辑界面（后续立项；届时才引入输入控件与键盘焦点问题）
- 文件系统实时监听 / 窗口焦点重扫（记 pending-decisions）
- 动态插件加载（dylib/进程外源）、源切换器 UI、Windows reveal
- gitignore 的字符类 `[]`、转义等超出 spec 子集的特性
- 跨重启的展开态持久化；`file_tree` 孤儿条目清理

## Decisions

### D1 模块落点：两个顶层模块

`src/workspace_property_source/`（`mod.rs` = trait + DTO + 静态注册表，`filetree.rs` = 首个实现）+ `src/tree/`（`TreePanel`）。删 `src/shell/placeholder.rs`，`shell/mod.rs` 换装。
**理由**：用户明确数据源须独立、后续插件化；面板与数据分居使面板对源无知。**替代**：单 `src/tree/` 模块（被否——概念权重与插件边界不够物理）。

### D2 边界契约：`WorkspacePropertySource` trait

```text
name()                      -> &str            // 面板标题与注册表 id 依据
refresh()                   -> ()              // part1: 定位 active ws.path + 重读配置规则 + 后台扫描
view()                      -> SourceNode      // part2: 面板拉取的过滤后视图
menu(item_id)               -> Vec<MenuDesc>   // 右键功能描述
execute(item_id, func_id)   -> Vec<SourceEffect> // 右键功能执行, 返回"效果意图"
open(item_id)               -> ()              // 左键钩子, v1 no-op (TODO 默认打开系统)
```

- 源持 `WeakEntity<TerminalApp>` 自行定位根路径（用户定的 part1 归属：path 定位 + 扫描都在源侧）。
- `execute` 返回 **`SourceEffect` 枚举**（`CopyText(String)` / `RevealPath(PathBuf)`）而非直接操作窗口/系统：剪贴板要 `&mut Window`、reveal 要进程——**效果由面板执行**，源保持可单测、可跨插件边界。
- **替代**：源直接拿窗口句柄写剪贴板（被否：污染边界、插件不可行）。

### D3 中立 DTO，适配器在面板侧

`SourceNode { id, label, kind: Dir|File, children }`、`MenuDesc { id, label }`、`SourceEffect`——**均不含 gpui 类型**。面板适配：`SourceNode → TreeItem`、`MenuDesc → PopupMenu`、`SourceEffect → 剪贴板/系统命令`。
**理由**：gpui 的 `TreeItem` 内嵌 `Rc<RefCell>` 状态，跨不过未来插件（FFI/dylib/进程）边界；现在换便宜。**替代**：`view()` 直接吐 `TreeItem`（被否，见前）。

### D4 展开态面板自持

面板维护 `expanded: HashSet<id>`，听 `TreeEvent::Expanded/Collapsed` 维护，构造 `TreeItem` 时贴 `.expanded(set.contains(id))`。
**理由**：刷新 = 重扫 = 全新 `Rc`（库内展开态必丢），过滤重建同理；状态放面板侧则**刷新、过滤、重建三者均不丢**，且 UI 状态不跨插件边界。**替代**：靠 `TreeItem` clone 共享 `Rc`（仅过滤成立、刷新失效）。刷新后对新树裁剪集合（陈旧 id 无害，裁剪只是整洁）。

### D5 过滤引擎：手写 glob 子集 → 正则，文件行限定

spec 承诺的子集：`*`（不跨 `/`）、`?`、`**`（跨 `/`）、路径模式相对 workspace 根、后写覆盖、`!` 前缀反悔。实现：模式逐条编译为正则，按序求值取最后命中；**目录行不参与求值**（结构保留，对 gitignore 的有意偏离）。
**理由**：子集手写约几十行，兑现 proposal 的"零新增 crate"；**替代**：引 `globset`/`ignore` crate（被否：超出 spec 的能力白买，字符类等反而制造未测行为）。
规则读取：`store.rs` 新增 `load_file_tree_rules(path)`；面板 ⟳ 与 refresh 时重读。

### D6 扫描：全量递归 + 排除表 + 后台线程

`read_dir` 递归，排除 `.git node_modules target build dist .DS_Store`（集合常量，spec 锚定），产出 `SourceNode` 权威树。扫描跑 `cx.spawn` 后台执行器，完成回调里 `panel.update` 换树。
**重扫粒度 = 根路径变化**：active 变但 root 相同（同 ws 切 agent）不重扫。
**替代**：按展开惰性扫（被否：未扫目录 `children` 空 → 全是假文件，且要付异步展开机器；记 pending 等真实大仓需求）。

### D7 刷新编排（两部分的落地）

```text
触发 (panel): 首次加载 / active 变化经 app 转发 / ⟳ 点击
  └> source.refresh()  [part1]  定位 root + 读规则 + spawn 扫描
        └> 完成回调: source 落权威树
  └> source.view()    [part2]  面板拉视图 → 适配 → set_items
```

- `app.rs` 把树面板加入 `notify_panels` 转发名单（active 变化要送达面板），沿用 `cx.defer` 防重入。
- refresh 内部顺序：读规则 → 扫描 → 落树 → 拉视图，一次回调内完成，展开集裁剪后贴回。

### D8 交互接线

- **左键**：`render_item` 返回的 `ListItem.on_click` → `source.open(id)`（v1 no-op）；选中/展开切换由库内置行为完成（两者共存为 spike 验证项，见 Risks）。
- **焦点**：面板**任意点击（行/按钮）后把焦点交回终端面板**——占位面板同款路径；v1 面板内无可聚焦文本控件，键盘唯一归宿仍是 active session，`agent-shell` 契约零修改。
- **右键**：`Tree::context_menu` 构建器按 index→entry→id 取 `source.menu(id)` → `PopupMenu`；项点击 → `source.execute` → 面板执行 `SourceEffect`（`CopyText` → `window.write_to_clipboard`；`RevealPath` → macOS `open -R` 子进程，`cfg(target_os = "macos")`）。
- **空目录图标**：`is_folder()` 对空目录为 false——面板从 `SourceNode` 遍历出 `dir_ids` 集合，`render_item` 据此选文件夹图标/展开 affordance，不依赖库的 children 推断。

### D9 面板 chrome 与注册

`TreePanel` 实现 `Panel`：`title_bar() = false`、`inner_padding() = false`（沿 D8 先例），自带 header（`source.name()` 标题 + ⟳ + 只读规则摘要 + `×`→`close_right_dock`）+ body（`Tree` 或空态/降级态）。注册必须 `panel_handle` 包装；`DockLayout` 三槽装配零改动。

### D10 `store.rs` 保全区段

`Persisted` 增 `file_tree: HashMap<String, Vec<String>>`（`#[serde(default)]` + 空时 skip序列化）；**`save` 改为 load→改 workspaces→写回**（现签名按 paths 重建会洗掉区段——workspace-management 修订场景的实现点）。旧文件缺字段照常解析（serde default），启动契约不变。

## Risks / Trade-offs

- **[库左键"选中+切换"与我们的 `on_click` 可能争用]** → spike：headless 断言单击文件=选中+open 钩子恰一次、单击目录=切换恰一次；若外层处理器吞事件，退路是把分发挂到 `ListItem` 内层（事件冒泡先内后外）。
- **[大仓全量扫描耗时]** → 后台线程 + 排除表兜底；仍超时的极端仓记 pending（惰性扫描）。
- **[`store::save` 漏保全 → `file_tree` 被洗]** → 专测"Filter section survives workspace mutation"；签名改造与读写往返同一提交完成。
- **[空目录渲染成文件]** → D8 `dir_ids` 集合修正图标；点击空目录展开为空属可接受表现（spec 未承诺其内容）。
- **[刷新丢展开]** → D4 面板自持，测试"Refresh preserves expansion"锚定。
- **[配置界面立项时键盘契约冲突]** → 已在 pending-decisions 记录：届时须回头修订 `agent-shell` 绝对条款（本轮靠"无输入控件"规避）。
- **[glob 子集与真 gitignore 行为差异（字符类/转义）]** → spec 只承诺 `*?**`/路径/顺序/`!`；超出部分不实现、不宣传。

## Migration Plan

无数据迁移：`file_tree` 区段可缺省（serde default），旧 `.workspaces.json` 原样可用；新面板替换占位属同仓源码演进，回滚 = revert 该提交。部署即本地 `cargo run`。

## Open Questions

- 过滤摘要在 header 的呈现密度（逐条列 vs 计数 `rules: 3`）——实现期定，不影响 spec 与任务拆分。
- 注册表 v1 的形态（`Vec<(id, factory)>` 常量即可）——切换器立项时再定型。

## 实现结论（apply 阶段回写，2026-11-09）

- **测试注入缝不能包在 `app.update` 租约内**：`set_source → on_data_changed
  → resolve_active_root` 会读 app → 重入 panic（"cannot read TerminalApp
  while it is already being updated"）。生产路径天然安全（`notify_panels`
  走 `cx.defer`、点击走监听器，均无 app 租约）；测试先在非更新态读出
  `tree` 实体再单独 update。
- **`texts()` 只收录 `.test_text()` 标注的元素**（pane F4 注释同款）：
  plain string 子节点不走记录路径 — header 规则摘要用 `.child(text) +
  .test_text(text)` 双写。
- **`.workspaces.json` 形状契约**：`Persisted.workspaces` 无 serde default
  （缺键 = 损坏文件，与 spec 的启动错误契约一致）— 任何该文件的测试夹具
  必须形状完整（带 `workspaces` 键）。
- **菜单项 id 是索引**（`Integer(n)`），与树行 id 撞名 — 测试点击须
  `within("popup-menu")` 限定子树。
- **左键共存由库保证**：gpui-base Tree 文档明确内容 on_click **先于**其
  内置"选中+切换"执行 — `file_click_invokes_open_once` 断言各恰一次。
- **树分支不是裸 notify**：`notify_panels` 对树调 `on_data_changed`
  （根比对 → 按需重扫 → 同步 `TreeState` → 重渲），其余面板仍裸 notify。
- **glob 引擎手写段匹配**（`*`/`?`/`**` + 递归）兑现零依赖承诺；字符类等
  超 spec 子集特性有意不实现（Risks 已记）。
- **测试必须隔离 CWD `.workspaces.json`**（gitignored，跑过一次 app 就会
  出现）：`TerminalApp::new` 经 `store::load()` 加载它，按位置索引
  (`workspaces[0]`) 的测试会被外来条目错位。`tree::tests::open_app` 起完
  先 `workspaces.clear()`。教训与上一 change 的 `inject_test_workspace`
  同族：**外部持久化状态是测试翻车的隐形变量**。

