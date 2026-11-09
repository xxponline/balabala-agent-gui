# Proposal

## Why

`TerminalApp`（`src/app.rs`，849 行）已成 god object：状态、渲染、事件、输入全在一起，
sidebar 与终端面板是自由函数且经 `Context<TerminalApp>` 反向耦合，事件载荷携带
`(ws_idx, agent_idx)` 索引在删除元素后即失效；同时界面没有任何"槽位"概念——下一个功能
模块需要一个右侧面板时无处安放。调研（`Docs/zed-ui-composition-research.md`）确认
gpui-kit 0.9 自带完整 `DockArea` 机制（拖拽 resize / 开合 / 面板注册全部内建），
现在是把界面拆成独立模块并接入 dock 机制的时机。

## What Changes

- **界面模块实体化**：sidebar、终端面板、右占位面板各收为一个 gpui entity，
  实现 gpui-component 的 `Panel` trait（`Render + Focusable + EventEmitter`），
  反向持有 `WeakEntity<TerminalApp>`；`TerminalApp` 瘦身为组合根
  （model + 外壳容器 + 事件漏斗 + IME 输入桥）。
- **接入 `DockArea` 布局**：左 dock（现有 sidebar 内容）、中心（终端面板，经
  `DockLayout::tabs()` 注册）、右 dock（占位面板）；面板 chrome 统一
  `title_bar() -> false` 以保住 spec 的"+ new workspace 置顶"语义。
- **新增（左 dock 可调宽）**：sidebar 所在左 dock 支持拖拽 resize
  （DockArea 内建 handle + clamp），左 dock 不可关闭（spec 可达性）。
- **新增（右 dock）**：默认渲染占位面板（header × + 提示 body），支持拖拽
  resize、`×` 关闭（零渲染；本轮不提供会话内重开路径，重启恢复默认）；
  占位面板即下个功能模块的接入样板。
- **身份 ID 化**：`active: Option<(usize, usize)>` → `Option<EntityId>`，
  `SidebarEvent` 枚举漏斗（`on_sidebar_event` 单一入口），事件载荷永不携带索引。
- **门禁 spike（临时分支，验证后丢弃）**：迁移前验证焦点 containment
  （Tab 转发 / IME 上屏 / 一步切焦点在 DockArea 嵌套下不回归）；失败则停止迁移，
  走预案（F1 焦点兜底 → F4 回退自研 Dock）后再重启。
- 行为保持：agent-shell / workspace-management spec 全部验收项不变
  （`cargo test`、`terminal-grid` 钩子、一步切焦点、Ctrl 不转发、中文 IME 等）。

## Capabilities

### New Capabilities

- `dock-layout`: 窗口左右 dock 槽位的行为——左 dock 承载 sidebar 且可拖拽调宽、
  不可关闭；右 dock 默认渲染、可拖拽调宽、可关闭（关闭期间零渲染，仅重启可
  恢复默认）；dock 尺寸变化时中心终端网格随可用面积重算列行。

### Modified Capabilities

（无——结构拆分与 PaneGroup 化属实现细节，既有 requirement 文本不变；
现有 spec 的焦点/输入/渲染行为在重构后必须逐项保持。）

## Impact

- **代码**：`src/app.rs`（瘦身，`TerminalApp::new` 增加 `window` 参数）、
  `src/ui/sidebar.rs` → `src/sidebar/mod.rs`、`src/ui/pane.rs` → `src/pane/mod.rs`、
  新增 `src/shell/{mod,placeholder}.rs`、`src/ui/mod.rs`（保留颜色映射）、
  `src/main.rs`（签名直传 window）。`agent_entities/`、`workspace/`、
  `tests/embed_layout.rs` 零改动。
- **测试**：`app.rs::adaptive_grid_dispatch` 需随签名/ID/导入路径更新；
  其断言为相对值，在新布局下已算账兼容。
- **依赖**：不新增 crate——`gpui-kit 0.9` 经 `gpui_kit::component::dock` 已含
  DockArea/DockSkin（底层 gpui-base）。
- **风险**：焦点逃逸会使 Tab/IME 同时失效（同一根因）——由门禁 spike 前置拦截，
  最坏回退自研 Dock（steps 1-3 的模块拆分不受影响）。
