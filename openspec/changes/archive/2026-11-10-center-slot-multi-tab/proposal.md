# Proposal

## Why

右槽文件树的左键 `open` 钩子自立项起就是 no-op，其 spec 原文写着 "deferring
to the future default-open system"——该系统至今未立项：用户点开一个 `.rs`
文件无处可看。同时中槽在结构上不具备承载第二块内容的能力（`DockLayout::tabs()`
里只注册了唯一面板 `TerminalPanel`）。本变更立项**默认打开系统**：中槽多页签化
+ 统一打开接口（封装对象 → handler 分发 → 去重开签），并落地首个 handler——
只读 Editor 查看 `.rs` 文件。

## What Changes

- **中槽单面板 → 页签组**：console（`TerminalPanel`）恒为第一签——
  `closable() = false` 已有，新签经 `add_panel_view(Center)` 尾插天然保持
  index 0；中槽顶栏常驻（Auto + console `title_bar=true`：单 console 签
  为 30px 标题栏, 多签为 tab 条；2026-11-10 headed 验收追加, 方案 B 定案,
  左右槽不受影响）；console 签带标题
- **统一打开接口（新模块）**：接受封装对象 `OpenObject{path, kind}`
  （来自右槽左键钩子）→ 静态 handler 注册表按序匹配：
  - 无命中 → no-op（目录行、非 `.rs` 文件行 v1 保持"仅选中"）
  - 命中 → 查 dedup 表（key = 绝对路径）：已有 → `select_panel` 聚焦
    **不重复开签**；没有 → 创建面板签 + 登记，签关闭经 `Panel::on_removed`
    摘除（保证关后可重开）
- **首个 handler：只读 `.rs` 查看器**：gpui-component 内建
  `Editor`（`EditorState::new(window, cx).language("rust")` +
  `set_readonly`），不自研编辑器；tab 标题 = 文件名；Cargo.toml 启用
  `gpui-kit/syntect` feature 提供语法高亮（不启用则黑白纯文本）
- **右槽钩子接入**：`WorkspacePropertySource::open` 从 no-op 变为构造
  `OpenObject` 交打开接口（part1/part2 分工与 `SourceEffect` 同款：
  源保持可单测、效果由面板/应用层落地）
- **spec `agent-shell` 焦点/键盘条款改写（BREAKING）**："main pane 永远
  渲染 active session / 应用失焦是键盘不转发的唯一例外" 的二元假设 →
  "键盘与 IME 跟随中心 active tab 所属面板；console 签激活时原条款逐字
  生效"；sidebar 选/建 agent SHALL 切回 console 签（一步切焦点经库内建
  `focus_active_panel` 保持）
- **spec `file-tree` 左键钩子条款落地**："v1 open action 无可见效果" →
  按 handler 结果开签/聚焦；"Keyboard stays with active session" 场景改写
  为跟随 active tab

## Capabilities

### New Capabilities

- `center-tabs`: 中心槽页签模型与默认打开系统——console 第一签不变量、
  封装对象的 handler 分发表、按绝对路径去重（重复打开 = 聚焦已有签）、
  首个只读 `.rs` 查看 handler、页签关闭与去重表生命周期

### Modified Capabilities

- `agent-shell`: "Session selection focus model"（main pane 改为渲染
  active tab；console 签渲染 selected session；点 sidebar agent 切回
  console 签）、"Keyboard input forwarding"（失焦唯一例外改为"active tab
  面板 + 失焦"双分支）、"Terminal output rendering"（渲染主体限定为
  console 签）、"Pane keyboard focus claim"（pane 焦点随 active tab）
- `file-tree`: "Left-click open hook"（v1 no-op → handler 命中时打开/聚焦
  中心页签，未命中保持仅选中）与场景 "Keyboard stays with active session"
  （改为键盘跟随中心 active tab 面板）

## Impact

- **新增**：打开系统模块（`src/property_detail_manager/`：`OpenObject`、handler 注册表、
  dedup 表、`TerminalApp::open_object` 接缝）、`EditorPanel`（只读查看
  面板实体）、对应无头测试
- **修改**：`src/shell/mod.rs`（console 签标题）、`src/app.rs`
  （open 接缝 + sidebar 选/建 agent 时 `select_panel(console)`）、
  `src/workspace_property_source/{mod,filetree}.rs`（`open` 钩子构造
  `OpenObject`）、`src/tree/mod.rs`（焦点交回目标改为 active tab 面板）、
  `Cargo.toml`（`gpui-kit` 加 `syntect` feature）
- **spec delta**：`agent-shell`（4 条改写）、`file-tree`（1 条改写）
- **依赖**：零新增 crate（Editor/highlighter 均在既有 gpui-kit 栈内）；
  **前置 change `right-dock-file-tree`（17/18）须先归档**——本变更修改
  其 `file-tree` spec，归档顺序倒置会让 delta 失去挂载点
- **测试**：现有回归保持绿；新增去重/第一签不变量/钩子分发/键盘跟随
  active tab 的无头测试
