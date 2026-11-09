# Design

## Context

`TerminalApp`（src/app.rs，849 行）持有全部状态并渲染整窗；sidebar/终端渲染是
自由函数，listener 经 `Context<TerminalApp>` 直调 app 方法；`active` 用
`(ws_idx, agent_idx)` 索引，闭包捕获在删除元素后过期。完整调研与机制分析见
`Docs/zed-ui-composition-research.md`（Zed 架构）与 `Docs/module-split-design.md`
（本设计的完整版，含源码级出处）。关键约束：

- gpui-kit 0.9 已含 `gpui_kit::component::dock`（DockArea/DockSkin，底层 gpui-base），
  拖拽 resize、toggle、clamp、面板注册全部内建，无需新依赖。
- agent-shell / workspace-management spec 的验收项必须逐项保持
  （一步切焦点、Ctrl 不转发、中文 IME、`+ new workspace` 置顶、`terminal-grid`
  测试钩子等）。
- IME/Tab 路由依赖焦点落在 TerminalPanel 的 focus handle 上（design D5 的
  spike 结论），DockArea 嵌套是新变量。

## Goals / Non-Goals

**Goals:**

- 界面模块各为一个 entity（实现 `Panel` trait），依赖单向：面板 →
  `WeakEntity<TerminalApp>`，永不反向 import。
- `TerminalApp` 瘦为组合根：model + 外壳容器 + `SidebarEvent` 漏斗 + IME 输入桥。
- 左 dock 承载 sidebar 可拖拽调宽、不可关；右 dock 占位面板默认渲染、可调宽、
  可关——本轮无会话内重开路径，重启恢复默认（spec 见
  `specs/dock-layout/spec.md`）。
- 迁移被门禁 spike 前置保护：焦点风险未验证前不动生产代码。

**Non-Goals:**

- 多面板同槽 tab 化、拖拽换位、面板拆分（DockArea 内建但不启用）。
- 布局持久化接线（`load`/`dump`，字段预留即可）、bottom dock、zoom、`ClosePanel` 流程。
- 状态栏 / titlebar / 覆盖层实现（外壳只留挂载位）。
- 右 dock 的会话内重开入口（快捷键/边缘把手）——随下个功能一起设计验收。
- 自研 `Dock`/`PaneGroup`（被 DockArea 取代）。

## Decisions

**D1 采用 gpui-component `DockArea`，不自研 dock。**
内建能力（resize handle + 窗口级 mouse tracker + clamp、`toggle_dock`、
`DockLayout`/`PaneTree` 未来分屏）省掉约整套自研机制；代价是面板强制 Entity 化。
备选"自研轻量 Dock"保留为回退方案（见 Risks F4）。
调研证据：递归 grep 两个 crate 的 dock 目录，key binding / key context /
on_key_down / input handler **全部为零**——dock 不可能劫持 Tab 或覆盖 IME handler。

**D2 面板 Entity 化，反向只持 weak。**
`SidebarPanel` / `TerminalPanel` / `RightPlaceholderPanel` 实现
`gpui_kit::component::dock::Panel`（= `Render + Focusable + EventEmitter<PanelEvent>`
+ `panel_name()` 等）。引用链 `TerminalApp → DockArea → 面板 →(weak)→ TerminalApp`
无强环。备选 plain struct 被 `Panel` trait 要求否决。

**D3 焦点/IME 归属：单一 focus handle。**
`TerminalPanel` 的 `Focusable::focus_handle` 即原 `pane_focus`，同时被
`#terminal-pane` 的 `track_focus` 挂载、被 `SelectAgent` 一步聚焦、被 dock 的
`focus_active_panel` handoff 命中（同一 handle，机制顺势）。IME 的
`EntityInputHandler` 实现留在 `TerminalApp`（trait 约束），面板在 canvas paint 内
`weak.upgrade()` 后注册 `ElementInputHandler`——输入桥代码**原样搬不重写**。
备选"面板实现 EntityInputHandler"增加迁移面，不取。

**D4 chrome：全面 `title_bar() -> false`。**
否则 DockSkin 标题栏压住 spec 置顶的 `+ new workspace`。附带
`closable() -> false` / `zoomable() -> false`（关的是 dock 不是 panel；不用 zoom）。
代价：内建 toggle 按钮（位于 title bar）不可见。chrome 策略已定案
（grill 确认）：全部 false，备选（center 保留 title bar 换内置按钮）不再考虑。

**D5 身份 ID 化 + 事件漏斗。**
`active: Option<EntityId>`（gpui entity id 即身份，`agent_entities/` 零改动；
Zed 同款先例 `preview_item_id`）；`SidebarEvent` 枚举收拢到
`TerminalApp::on_sidebar_event` 单入口。listener 回调式不变（design D7 理由保留），
仅载荷从索引换成 `PathBuf`/`EntityId`，闭包捕获不再过期。

**D6 关闭语义用 `toggle_dock`，不用 `ClosePanel`。**
`toggle_dock` 只切 `is_open`：零渲染、面板实体保留——下个功能补上会话内
重开入口时零成本。本轮 `×` 关闭后无重开路径，重启恢复默认（grill 定案）。
`ClosePanel` 会移除面板并 normalize 掉 dock，需重新注册，不用
（占位面板 `closable=false` 也挡住它）。

**D7 尺寸交给内建。**
播种 `add_panel_view(panel_handle(...), ..., Some(px(240.)))`（= 现 sidebar 宽；
**panel_handle 必须**，裸 entity 绕过 title_bar 判断，spike §10.5）；clamp 内建
（min 100px，max = 窗宽 − 100 − 对侧）。自定义常数不引入，视觉太窄再调（Q5）。

**D8 迁移顺序：门禁在前，实体化借"面板可独立渲染"增量走。**
`Panel: Render` 使面板实体可先作为普通 child 挂进现有布局——步骤 2/3 不碰
DockArea、不碰视觉/测试，步骤 4 才做纯装配切换（一次 render 替换）。

## Risks / Trade-offs

- **[高] 焦点逃逸 → Tab/IME 同时失效（同一根因）** → 门禁 spike（临时分支，
  验证后丢弃）前置：无头 T1-T5/I1（每步断言 `window.focused() == pane handle`）
  + headed I2/T6（真实中文 IME）。失败预案分级：F1 焦点兜底（`set_active` +
  `on_click` 显式聚焦）→ F2 绑定加固 → F3 定点修 IME → F4 回退自研 Dock。
  steps 1-3 不依赖 DockArea，**F4 回退不伤已交付的模块拆分**。
- **[中] 失效链断裂（面板化后输出不刷新）** → spike V3 实测（Docs §10.5）：
  面板**自读不触发重建**，非视图 notify 只重建根视图——必须**显式 notify
  面板**：drain 循环（session 产出 → 面板）+ model 漏斗转发（tasks 3.3）；
  面板 render 仍须每帧现读防缓存。notify 面板即自动重绘已实测 ✓。
- **[中] gpui-component dock 是 0.9 新模块，0.10 可能变** → 版本锁 Cargo.toml；
  对外只用 `component::dock` 再导出面。
- **[低] DockSkin 像素差异破坏 `adaptive_grid_dispatch` 断言** → 断言均为
  相对值（`bounds < 800` 在左 240 + 右 240 下中心 ≈500 已算账兼容）；
  只调测试窗口尺寸不改断言。
- **[低] 无头测试下 DockSkin 表现未知** → dock 自带 `test_support`；
  不行则改"装配后 bounds 断言"变体。

## Migration Plan

每步可编译、`cargo test` 全绿（详见 tasks.md）：

1. **门禁 spike**（临时分支）：焦点 containment T1-T6/I1-I2 + 顺带 V1-V6；
   全过才继续，失败则停下走预案。
2. ID 化（`active: Option<EntityId>` + 事件载荷去索引）——与 DockArea 无关。
3. `SidebarPanel` 实体化（先挂现有布局）。
4. `TerminalPanel` 实体化（焦点/键/IME 桥迁入，先挂现有布局）。
5. 装配切换：`TerminalApp::new(window, cx)` + `shell::build`，render 换为
   外壳 + `dock_area`，右占位注册，chrome 配置。
6. `ui/widgets.rs` 拆分（`TextTooltip` 归位，已确认本轮做）。

**回滚**：步骤 5 是单点切换，git revert 即回老布局；步骤 3/4 的实体作为普通
child 渲染，revert 同样干净。最坏 F4：steps 1-4 保留（模块拆分照常交付），
仅装配层换成自研 Dock。

## Open Questions

- Q5 内建 100px 最小宽视觉是否够——先用内建，太窄再上自定义 clamp。

**已定案（grill 确认，2026-11-08）**：Q1 重开键位——不需要，右 dock `×` 关闭
后仅重启恢复；Q4 chrome——全部 `title_bar() -> false`；右 dock 关闭不持久化、
重启回默认宽；门禁保留 headed I2/T6；`ui/widgets.rs` 本轮建。
