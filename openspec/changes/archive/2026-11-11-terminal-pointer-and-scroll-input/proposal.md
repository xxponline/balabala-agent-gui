# Proposal

## Why

pi-shell-rs 侧的三个 change（`add-message-scrollback`、`add-mouse-wheel-scroll`、
`add-tool-card-click`）已归档：TUI 内部具备整页回看（PageUp/PageDown）、
滚轮行滚动、工具卡片点击展开/折叠的完整能力，入口都是 `UiEvent`
（`Wheel(i16)` / `Click{x,y}` / `Key`）。但嵌入宿主（本 GUI）送不进去——
`EmbeddedApp` 只暴露 `inject_key`/`inject_text`，GUI 的 `keystroke_to_keyevents`
也丢弃 pageup/pagedown，`TerminalPanel` 更是完全没有滚轮与点击坐标处理。
本变更打通最后这一段：宿主侧的指针/滚动输入转发。

## What Changes

- **PageUp/PageDown 键转发**：`keystroke_to_keyevents` 补 `"pageup"`/`"pagedown"`
  → `KeyCode::PageUp/PageDown`（gpui 键名已确认）；pi 侧 `KeyIntent::ScrollPage`
  已消费，零 pi 逻辑改动
- **pi-shell-rs 纯门面加法**（不碰逻辑）：`EmbeddedApp::inject_wheel(lines: i16)`、
  `EmbeddedApp::inject_click(x: u16, y: u16)`（各一行 send）+ `mod scroll` 改
  `pub mod scroll`（宿主引 `ALT_WHEEL_MULTIPLIER`，避免字面 5 漂移）
- **`AgentEntity` trait 扩展**：`send_wheel(lines: i16)` / `send_click(x, y)`
  （与 `send_key`/`send_text` 并排；测试假实现跟改）
- **TerminalApp 分发**：`send_wheel`/`send_click` → active session +
  诊断计数（`tab_forwards` 先例的观测口）
- **TerminalPanel 滚轮**：`.on_scroll`——`ScrollDelta::Pixels` 余数累积
  ÷ `line_h` 凑整行、`Lines` 直通行数、`modifiers.alt` × `ALT_WHEEL_MULTIPLIER`、
  `stop_propagation`（内层先冒泡，dock 的 `tab-content` 滚动容器抢不走）；
  resize 换行高时清余数。**v1 不做速度加速**（`WheelAccel` 输入模型是
  notch 流，与触控板像素流错位——留作 v2 现成材料）
- **TerminalPanel 点击**：`.on_click` 现有聚焦逻辑上追加坐标转发——窗口坐标
  − `terminal-grid` origin（复用 `propagate_grid_size` 的 `cx.defer` 管道存
  origin）÷ `(cell_w, line_h)` → 0-based `(col, row)`；越界（状态条/边框）
  不发。卡片命中与展开/折叠语义归 pi-shell-rs（其 spec 已覆盖），本侧只管转发
- **spec `agent-shell` delta**：键盘清单扩 PageUp/PageDown（**BREAKING** 语义
  变化：原清单是封闭枚举）；ADDED 滚轮系列（历史滚动 / 悬停归属与截断 /
  像素累积 / Alt 倍率）与 ADDED 点击坐标转发共五条需求

## Capabilities

### New Capabilities

（无——滚轮/点击/翻页都是"终端面板转发用户输入"这一持久行为的延伸，
归入既有 `agent-shell`；`center-tabs`/`file-tree` 不受影响。）

### Modified Capabilities

- `agent-shell`: 「Keyboard input forwarding」转发清单扩 PageUp/PageDown；
  ADDED 四条滚轮需求（历史滚动、悬停归属与截断、像素累积、Alt 倍率）+
  ADDED「终端区域点击坐标转发」（0-based 单元格坐标、越界不发、
  卡片行为端到端验收）

## Impact

- **修改（pi-shell-rs, `ref/` 副本）**：`src/app/embed.rs`（+2 inject 门面）、
  `src/app/mod.rs`（`mod scroll` → `pub mod scroll`）；纯加法，无逻辑改动
- **修改（本仓库）**：
  - `src/agent_entities/mod.rs`（trait +2 方法）+ `pi_agent_shell.rs` 实现
  - `src/app.rs`（`send_wheel`/`send_click` 分发 + 诊断计数）
  - `src/pane/mod.rs`（键映射 2 行、`.on_scroll`、`.on_click` 扩展、
    grid origin 存取、滚动累积器）
  - 测试假实现（FakeAgent/CountingAgent）跟改
- **spec delta**：`agent-shell`（1 MODIFIED + 2 ADDED）
- **依赖**：零新增 crate；行高/origin 工具复用 `grid_metrics`/defer 管道
- **测试**：无头（`window.scroll`/`window.click` 注入口已有）+ headed 真机
  验收（点卡片展开、慢滚逐行、Alt 快翻、viewer active 时 console 无事件）
