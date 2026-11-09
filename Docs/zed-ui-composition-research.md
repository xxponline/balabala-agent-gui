# 调研：Zed 主界面与子面板的组合方式

> 来源: `ref/zed` (软链的 zed 代码库)，对照本项目 `src/`
> 目的: 探索本项目界面上的各个模块（sidebar、终端面板、未来新增面板）应如何组织

---

## 一、Zed 的主界面组合方式

### 1. 布局层：Workspace 实体拥有所有槽位

`Workspace` struct（`crates/workspace/src/workspace.rs:1577`）直接**持有**各区域的实体：

```text
Window
  |
  +-- Workspace (Entity) ...................... crates/workspace
       |
       +-- titlebar_item: Option<AnyView>  <-- 注入点: app 层塞进来的
       |
       +-- center: PaneGroup ................. 递归分屏树
       |     Member::Pane(Entity<Pane>)       Pane 内是 tab 列表
       |     |      tab = Box<dyn ItemHandle>  (editor / terminal / ...)
       |     Member::Group(Axis, [Member])    可继续 split
       |
       +-- left_dock:   Entity<Dock> -+
       +-- bottom_dock: Entity<Dock> -+--> Dock 持有 Vec<PanelEntry>
       +-- right_dock:  Entity<Dock> -+      active_panel_index (类似 tab)
       |
       +-- status_bar: Entity<StatusBar> .... 若干 StatusItem 槽位
       +-- modal_layer / toast_layer ....... 覆盖层, 不进布局流
       +-- project / active_call / ... ..... 业务状态
```

render 函数（`workspace.rs:9777`，`impl Render for Workspace`）就是纯布局拼装：
titlebar 区 → `[left dock | center | right dock]` → bottom dock → status bar。
dock 的 resize/drag 都在壳层处理，**面板自己完全不知道自己被放在哪个 dock**。

### 2. 扩展点：四个 trait 就是全部契约

| trait | 槽位 | 实现方（feature crate） |
|---|---|---|
| `Panel`（`dock.rs:36`） | 左/下/右 dock | project_panel, terminal_panel, agent_ui::agent_panel, debug_panel… |
| `Item`（`item.rs:170`） | center 的 tab | editor, terminal, agent_diff… |
| `StatusItemView`（`status_bar.rs:42`） | status bar | activity_indicator, breadcrumbs… |
| `ToolbarItemView`（`toolbar.rs:24`） | center 顶部 toolbar | breadcrumbs, search… |

`Panel` trait 的关键设计（`dock.rs:36-104`）：

- `persistent_name()` / `panel_key()` — **稳定身份**：持久化按 key 存尺寸/位置，
  序列化逻辑与 UI 完全解耦
- `position()` / `position_is_valid()` / `set_position()` — 面板声明自己的合法位置，
  dock 壳层负责布局执行
- `default_size()` / `min_size()` / `initial_size_state()` — 尺寸策略由壳层统一执行
  （`Dock::clamp_panel_size`）
- `toggle_action() -> Box<dyn Action>` — 面板自带开关 action，命令面板/快捷键免费获得
- `activation_focus_handle()` / `Focusable` — 焦点管理是 trait 的一部分，
  激活、焦点 containment 都由壳层按契约处理
- `PanelHandle`（`dock.rs:106`）— 类型擦除后的句柄，壳层只跟句柄打交道，
  **永远不 import 具体面板类型**

### 3. 注册：组合根在最外层 app crate

`crates/zed/src/zed.rs:788`：

```rust
async fn add_panel_when_ready(...) { workspace.add_panel(panel, window, cx); }
add_panel_when_ready(project_panel, ...),
add_panel_when_ready(terminal_panel, ...),
add_panel_when_ready(git_panel, ...),
...
```

`Workspace::add_panel<T: Panel>(entity<T>, window, cx)`（`workspace.rs:2837`）
统一处理：焦点挂钩、按 `panel_key()` 恢复持久化尺寸、投递到对应 dock。

依赖方向单向，workspace 永远不知道 project_panel 的存在：

```text
project_panel / agent_ui / terminal_panel   (功能 crate, 各自内聚)
        |  实现 trait, 依赖
        v
    workspace (壳: Workspace/Dock/Pane/Item/Panel) ---> ui (设计系统)
        |                                               |
        +-----------------> gpui <----------------------+
```

### 4. 逻辑与 UI 的管理方式

**(a) 状态 = 每模块一个 Entity。** gpui 的所有权模型
（`crates/gpui/src/_ownership_and_data_flow.rs`）：所有状态归 `App` 拥有，
模块间只传 `Entity<T>` 句柄，访问要 `update(cx, ...)`。跨模块通信只有三种：

- `cx.emit(Event)` + `cx.observe/subscribe` — 事件上抛 / 订阅
- `WeakEntity<Workspace>` 存在面板里 — 向上借用壳能力
- `actions!` + key context — 键盘命令分发

没有全局消息总线，也没有互相持有可变引用。

**(b) 模块内部：render 薄、方法厚。** 以 project_panel（`project_panel.rs`，8.4k 行单文件）为例：

```text
impl Render for ProjectPanel       <- 薄: 只做布局
    fn render() { ... self.render_entry(...) ... }
fn render_entry(...)               <- 私有渲染子函数, 返回 element
fn render_folder_elements(...)
impl Panel for ProjectPanel        <- 壳层契约适配层
事件: cx.listener(|this, e, ..| this.some_method(...))  <- 直调自身方法
```

事件处理是 `cx.listener` 闭包直调 `self` 方法；逻辑（selection、展开、拖拽状态）
都在 struct 自己身上。**渲染函数是纯函数式拼 element，状态变更走方法**。

**(c) 设计系统无逻辑。** `crates/ui` 全是哑组件 + style trait
（`Toggleable`, `Disableable`, `Clickable`, `VisibleOnHover` 等，见 `ui/src/traits/`），
任何业务状态都不进 ui crate。业务面板各自组合这些组件。

---

## 二、深入：Dock 与 Pane 的持有结构与接口

> 出处：`crates/workspace/src/{dock.rs, pane.rs, pane_group.rs, item.rs}`

### 2.1 两者是什么

| | Dock | Pane |
|---|---|---|
| 角色 | 窗口**边缘**的面板槽位（左/右/下） | **tab 容器**，split 树的叶子 |
| 一句话 | "这个边缘位置放哪个面板" | "这组 tab 显示哪个 item" |

两者结构上同形（列表 + 激活索引 + 焦点句柄），但回答的问题正交：
Dock 回答 *位置与开合*，Pane 回答 *tab 组织*。关键证据是它们可以互相嵌套
（见 2.6）。

### 2.2 持有结构

**Dock**（`dock.rs:283`）：

```rust
pub struct Dock {
    position: DockPosition,              // Left / Right / Bottom
    panel_entries: Vec<PanelEntry>,      // 面板表 (多个, 激活一个)
    workspace: WeakEntity<Workspace>,    // 向上借用壳能力
    is_open: bool,                       // 整体开合
    active_panel_index: Option<usize>,   // 当前可见面板
    focus_handle: FocusHandle,
    restoration: DockRestoreState,       // 持久化恢复状态机
    zoom_layer_open: bool,
    modal_layer: Entity<ModalLayer>,
    _subscriptions: [Subscription; 2],
}

struct PanelEntry {
    panel: Arc<dyn PanelHandle>,         // 类型擦除, 壳层不 import 具体面板
    size_state: PanelSizeState,          // { size: Option<Pixels>, flex: Option<f32> }
    _subscriptions: [Subscription; 4],
}
```

**Pane**（`pane.rs:399`，字段摘录）：

```rust
pub struct Pane {
    items: Vec<Box<dyn ItemHandle>>,     // tab 列表 (类型擦除)
    active_item_index: usize,
    activation_history: Vec<...>,        // 激活顺序 (MRU)
    preview_item_id: Option<EntityId>,   // 单次预览 tab (点文件不真打开)
    nav_history: NavHistory,             // 前进/后退栈
    toolbar: Entity<Toolbar>,            // tab 栏上方的 toolbar
    focus_handle: FocusHandle,
    workspace: WeakEntity<Workspace>,
    project: WeakEntity<Project>,
    // 注入式行为钩子 (由 Pane 的使用者设置, Pane 不知道是谁在用):
    can_split_predicate: Option<...>,
    should_display_tab_bar: Rc<dyn Fn(...) -> bool>,
    render_tab_bar: Rc<dyn Fn(...) -> AnyElement>,
    pinned_tab_count: usize,
    zoomed: bool,
    // ...
}
```

### 2.3 主要功能

**Dock**：

- 面板登记/移除（`add_panel` 按 `panel.position()` 自动路由到对应 dock）
- 开合（`set_open`）、激活（`activate_panel`）、面板间切换
- 尺寸策略：每面板持久化 `size`/`flex`，drag 调整、双击 reset、
  窗口变化 `clamp_panel_size`
- 缩放（`set_panel_zoomed` / `zoom_out`）
- KeyContext 派发 + `PanelButtons`（status bar 上的 dock 开关按钮组）
- 持久化恢复（`DockRestoreState`：先记 pending 再应用，防止恢复竞态）

**Pane**：

- tab 生命周期：`add_item` / `close_*` / `activate_item` / `swap_item_left/right`
- 预览 tab（`preview_item_id`：单次预览，切走即关）+ pin
- 导航历史（`nav_history`：backward/forward + tag 栈）
- 分屏（`split`、拖拽 `handle_drag_move`/`handle_tab_drop` 到任意方向）
- 缩放（`toggle_zoom` / `zoom_in` / `zoom_out`）
- 关闭策略（`close_other_items` / `close_clean_items` / `SaveIntent` 保存确认）
- toolbar 挂载、tab 栏定制（`set_render_tab_bar` 等注入钩子）
- render 上绑定大量 `actions!`（split/close/zoom/navigate 全部走 action 分发）

### 2.4 public 接口分组（关键组摘录）

**Dock（35 个 pub fn）**：

| 组 | 接口 |
|---|---|
| 查询 | `position` `is_open` `panels_len` `active_panel_index` `visible_panel` `active_panel` `zoomed_panel` `panel<T>` `panel_for_id` |
| 变更 | `set_open` `activate_panel` `add_panel` `remove_panel` `set_panel_zoomed` `zoom_out` |
| 尺寸 | `active_panel_size` `stored_panel_size_state` `set_panel_size_state` `toggle_panel_flexible_size` `resize_panel_sizes` `reset_panel_sizes` `clamp_panel_size` |

**Pane（110 个 pub fn，按组）**：

| 组 | 接口 |
|---|---|
| items | `add_item` `items` `items_len` `active_item` `item_for_path` `index_for_item` |
| 激活 | `activate_item` `activate_next/previous/last_item` |
| 关闭 | `close_active_item` `close_item_by_id` `close_other_items` `close_all_items` `close_items_to_the_side_by_id` `close_clean_items` |
| 导航 | `nav_history` `can_navigate_backward/forward` `navigate_backward` `go_to_older/newer_tag` |
| 预览 | `preview_item_id` `unpreview_item_if_preview` `close_current_preview_item` `set_pinned_count` |
| 分屏/缩放 | `split` `toggle_zoom` `zoom_in/out` `handle_tab_drop` |
| chrome | `set_render_tab_bar` `set_should_display_tab_bar` `toolbar()` |

### 2.5 对比表

| 维度 | Dock | Pane |
|---|---|---|
| 持有 | `Vec<PanelEntry>`（`Arc<dyn PanelHandle>` 擦除） | `Vec<Box<dyn ItemHandle>>` 擦除 |
| 激活 | `active_panel_index`（面板间切换） | `active_item_index`（tab 间切换） |
| 可见性 | `is_open` 整体开合 | tab 恒显示；预览 tab 临时 |
| 尺寸 | 每面板 `size/flex` 持久化 + drag | 由 `PaneGroup` 分割比例决定 |
| 稳定身份 | `panel_key`（字符串，持久化用） | `EntityId` |
| 焦点 | `focus_handle` + KeyContext | `focus_handle` + KeyContext + action 绑定 |
| 事件 | `PanelEvent {ZoomIn,ZoomOut,Activate,Close}` | `Event {AddItem,Remove,Split,Focus,...}`（14 种） |
| 向上引用 | `WeakEntity<Workspace>` | `WeakEntity<Workspace>` + `WeakEntity<Project>` |
| 行为钩子 | 少（位置/尺寸/图标声明式） | 多（tab 栏渲染、能否 split、拖拽判定均注入） |

### 2.6 嵌套关系（关键发现）

```text
Workspace
 +-- left/right/bottom_dock: Entity<Dock>
 |     +-- panel_entries[i]: PanelEntry { Arc<dyn PanelHandle>, size_state }
 |           +-- 例: TerminalPanel (impl Panel, terminal_view crate)
 |                 +-- center: PaneGroup   <-- dock 面板内部又是 Pane 树!
 |                 +-- active_pane: Entity<Pane>
 +-- center: PaneGroup
       +-- root: Member::Axis(...) | Member::Pane(Entity<Pane>)
             +-- Pane
                   +-- items: Vec<Box<dyn ItemHandle>>  (editor / diff / ...)
                   +-- toolbar: Entity<Toolbar>
```

**Pane 被复用两次**：既是 center split 树的叶子，也是 dock 面板
（如 `TerminalPanel`，`terminal_view/src/terminal_panel.rs:77`）内部的 tab
容器。`Panel::pane()` 让壳层能从面板里挖出 Pane（协作跟随、leader
边框等场景需要）。Panel 与 Pane 是两个正交抽象：

- **Panel** = "放哪、怎么在边缘显示"（位置/尺寸/图标/开合）
- **Pane** = "tab 怎么组织"（增删/激活/预览/分屏/导航）
- 复合面板 = Panel 实现内部再装 Pane/PaneGroup（`Dock > Panel > Pane > Item`）

### 2.7 对本项目的启示

我们目前是 `TerminalApp > Vec<Workspace> > AgentSession`，单 pane 无 tab：

1. **"一个 session 多视图"（终端/线程/日志）→ 对应 Pane 的 tab 能力**，
   但可以砍到最小集：`Vec<View>` + `active_idx` 即可，preview/nav-history/
   pinned 全套不需要。
2. **"边缘面板"（如右侧 agent 详情）→ 对应 Dock**，保持"出现第二个
   边缘面板时再抽 Dock-lite"的原判断；Dock-lite 的最小接口
   = 位置 + 开合 + 激活 + 尺寸持久化。
3. **"面板里再装容器"的复合模式值得记住**：面板不必是单叶视图，
   `TerminalPanel` 就是 "dock 里的 tab 终端群"。若未来做"会话组"，
   这是现成的成熟形态。
4. **Pane 的注入钩子模式**（`should_display_tab_bar` / `render_tab_bar` /
   `can_split_predicate`）比 trait 更轻：容器默认行为 + 使用者按需覆盖，
   不必为每种定制开一个 trait 方法。

---

## 三、对照本项目现状

```text
TerminalApp (src/app.rs, 849 行) ........ God object
  |-- workspaces: Vec<Workspace>     所有状态在这
  |-- active: Option<(usize,usize)>  索引式身份
  |-- 渲染: ui::sidebar::render(&self, cx) -> impl IntoElement   (自由函数)
  |          ui::pane::terminal_grid(screen, window)              (自由函数)
  |-- 事件: sidebar 里 cx.listener 直调 TerminalApp::create_agent 等
  |-- 另有: 输入映射、drain 循环、对话框、持久化...
```

差距不是"代码质量"问题，而是**未来加面板时的耦合点**：

1. **自由函数 + `Context<TerminalApp>` 反向耦合**：`ui/sidebar.rs` 的每个 listener
   都在调 `TerminalApp` 的方法。加第 3 个面板时，所有渲染函数都要认识
   `TerminalApp`，签名越拉越长。
2. **索引即身份**：`active: (usize, usize)`、事件里传 `ws_idx/agent_idx`。
   Zed 从头到尾用 `EntityId`/稳定 key。索引在删除元素后瞬间失效——
   listener 是预构建闭包，捕获了过期 idx 是真实隐患。
3. **没有"槽位"概念**：sidebar 内嵌 workspace 列表 + agent 列表，pane 内嵌
   grid + status bar，都是硬编码。下一个面板（命令面板？会话详情？多 tab？）
   只能继续往 `TerminalApp::render` 里塞。

规模对比：本项目 ~2.3k 行自有代码，Zed 光 workspace crate 就 54k 行。
**照搬 trait 动物园是过度设计。**

---

## 四、建议的演进方向（比例适当的 Zed 化）

### 短期（下一个功能到来前就值得做）

```text
src/
  app.rs ............ 组合根: 开窗 + 布局拼装 + 模块间接线
  ui/
    theme.rs ........ 调色板/颜色映射 (现在 mod.rs 里的)
    widgets.rs ...... TextTooltip 等共享小件
  sidebar/
    mod.rs .......... Sidebar 状态 + render (workspace/agent 列表)
  pane/
    mod.rs .......... TerminalPane 状态 + render (grid/focus/输入)
  agent_entities/ ... 不变
  workspace/ ........ 不变
```

三个具体动作，都只动结构不动行为：

1. **每个界面模块收成一个 struct**：拥有自己的状态切片 + `render()` +
   事件处理方法。`TerminalApp` 从"状态+渲染+逻辑"瘦身为"组合根 +
   模块间导航（active 切换）"。
2. **索引换稳定 ID**：给 session 一个 `SessionId`（递增即可），workspace
   用路径做 key（spec 里已是 unique identity）。事件携带 ID 而非 idx，
   闭包捕获 ID 就不会过期。
3. **上抛事件用小枚举**，替代 listener 直调 app 方法：

```text
SidebarEvent::NewWorkspace
SidebarEvent::SelectAgent(SessionId)
SidebarEvent::CloseAgent(SessionId)
        |  cx.emit / 闭包回传
        v
TerminalApp::handle_sidebar_event(...)   <- 唯一需要互相知道的地方
```

### 中期（出现"第二种同类面板"时才做）

- **引入 `Panel`-like trait**：当真要加第二个 dock 面板或第二种 center 视图
  （日志视图、thread 视图）时，再抽 `IconPanel`/`CenterItem` 之类的小 trait。
  trait 的价值在**可插拔 + 稳定身份 + 尺寸策略集中**，一个实现时不值得。
- **命令化**：如果要做命令面板/全局快捷键（pending-decisions 里已出现该信号），
  把动作从方法调用改成 Zed 式 `actions!` 分发，届时 SidebarEvent 枚举
  可自然升级成 action。

### 刻意不学

- 三 dock + 递归 split 树 — 本项目单 pane，PaneGroup 是几百行复杂度换没有的需求
- 250 crate 分层 — 单 crate 内 `mod` 分组足够
- `ItemHandle`/`PanelHandle` 类型擦除层 — 一两个实现时是纯噪音

---

## 五、遗留问题（按影响排序）

1. **下一个界面模块是什么？** 命令面板、多 tab 终端、会话输出搜索——
   不同的下一个决定 trait 要不要提前抽。如果近期只是加"输出搜索框"，
   短期三步就够。
2. **`active: (usize, usize)` 现在有没有咬过人？**（删除 workspace/session
   后界面跳错）如果有，ID 化可以提前到立刻做。
3. **sidebar 的布局要不要变成可配置的？**（比如 agent 面板未来挪到右 dock，
   像 Zed 的 agent_panel 那样）如果"面板位置可变"在路线图上，
   槽位抽象就值得更早认真做。
