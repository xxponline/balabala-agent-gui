# Tasks

## 1. pi-shell-rs 注入门面（`ref/pi-shell-rs`，纯加法）

- [x] 1.1 `EmbeddedApp` 加 `inject_wheel(lines: i16)` 与
  `inject_click(x: u16, y: u16)`（各一行 `keys.send(UiEvent::Wheel/Click)`），
  `src/app/mod.rs` 的 `mod scroll` 改 `pub mod scroll`；验证 pi 仓库
  `cargo test` 通过，且新增无头测试（建 channel → inject → 收到
  `UiEvent::Wheel`/`UiEvent::Click` 载荷一致）绿

## 2. GUI 契约与分发

- [x] 2.1 `AgentEntity` trait 加 `send_wheel(lines: i16)` /
  `send_click(x: u16, y: u16)`（`send_key` 旁，文档注明坐标系 = 会话帧
  0-based）；`PiAgentShell` 直通 `embed.inject_*`；FakeAgent/CountingAgent
  等测试假实现跟改；验证 `cargo check --all-targets` 通过
- [x] 2.2 `TerminalApp::send_wheel` / `send_click`：active session 分发 +
  `wheel_forwards` / `click_forwards` 诊断计数（`tab_forwards` 同款）；
  验证无头测试 `wheel_and_click_reach_active_session`（计数与载荷断言，
  无 active session 时不 panic 不计数）
- [x] 2.3 `keystroke_to_keyevents` 补 `"pageup"` → `KeyCode::PageUp`、
  `"pagedown"` → `KeyCode::PageDown`；验证 pane 无头单测
  `page_keys_map_to_crossterm_paging`（映射含/不含修饰键两个断言）

## 3. TerminalPanel 滚轮

- [x] 3.1 grid origin 入面板：`#terminal-grid` 挂
  `on_children_prepainted` 直读子元素窗口原点存入 `TerminalPanel`
  （prepaint 期不得 update 实体 → `Rc<Cell>` 裸闭包；实现揭示：canvas
  prepare bounds 的 y 是内容坐标，不可用）；验证无头测试
  `grid_origin_tracks_terminal_grid_bounds`（render 后字段 =
  `terminal-grid` 观测 bounds 的 origin 且非零）
- [x] 3.2 滚轮链路：`WheelAccumulator` 累积器 + `wheel_lines_for` 纯函数
  （Pixels÷行高、Lines 直通、trunc 扣余、line_h 变化清余、Alt×
  `ALT_WHEEL_MULTIPLIER`）+ `.on_scroll` handler（无条件
  `stop_propagation` → `app.send_wheel`）；验证 pane 纯测
  `wheel_lines_for_accumulates_alt_and_resets` 与窗口级
  `wheel_accumulates_pixels_through_panel`（慢滚 3 次出行、5 行一次发、
  入口计数；实现揭示：原 canvas defer 改 `on_children_prepainted` 入
  面板字段，测试名按可达链路命名）
- [x] 3.3 hover 归属验证：viewer active 时滚 `terminal-pane` 无 wheel 到
  session（console 不渲染 → handler 不存在）；无头测试
  `wheel_unreachable_while_viewer_active`（viewer 开启后 scroll →
  `wheel_forwards` 不变）

## 4. TerminalPanel 点击坐标

- [x] 4.1 `.on_click` 扩展：窗口坐标 − `grid_origin` ÷ `(cell_w, line_h)`
  → `(col, row)`，越界（负值 / ≥ cols/rows）不发；保留现有一步聚焦；
  验证无头测试 `click_inside_grid_delivers_cell_coordinate`（已知网格度量
  下点 grid 中心 → 期望 (col,row)）与 `click_outside_grid_sends_nothing`
  （`click_at` 点 grid 上方的状态条区域 → `click_forwards` 不变）

## 5. 滚动方向与快速 drain（headed 验收追加, design D7）

- [x] 5.0 滚动方向适配：`wheel_lines_for` 把 gpui `ScrollDelta` 的 y 符号
  翻转成 pi 约定（负=上滚看更早, standalone crossterm ScrollUp→-1 金标准,
  2026-11-10 验收反馈触摸板方向反了）；pane 纯测与窗口级
  `wheel_accumulates_pixels_through_panel` 符号期望同步, 全量绿
- [x] 5.4 `poll_sessions` 返回"是否消费到新帧" + 新增
  `_input_drain_task` burst（输入后 8ms×4 拍, 取到帧即停, 新输入重置）；
  `send_key`/`send_text`/`send_wheel`/`send_click`/`propagate_grid_size`
  尾部 kick；验证无头测试 `input_kick_drains_frames_before_main_tick`
  （测试 app 不启主循环 → 输入后帧只能来自 burst, run_until_parked 后
  断言帧已消费）+ 现有全量回归绿

## 6. 集成回归

- [x] 6.1 全量回归：本仓库 `cargo test` 全绿 + `cargo check --all-targets`
  0 warning；pi 仓库 `cargo test` 全绿
- [x] 6.2 headed 真机验收并记录：PageUp/PageDown 整页回看、鼠标慢滚逐行
  （**方向修正后**）、快滚成片、滚动无卡顿感（burst 生效）、Alt 快翻、
  点工具卡片展开/折叠、点状态条无反应、viewer active 时滚轮/点击不影响
  console 会话、dock 外层不误滚
- [x] 6.3 `openspec validate terminal-pointer-and-scroll-input --strict`
  通过（无新告警）

## Workflow follow-up

- 两仓分别 commit：`ref/pi-shell-rs`（门面，其独立 git 历史）+ 本仓库
  （feat 实现 + docs(openspec) 归档）
- review 后 `/opsx-archive terminal-pointer-and-scroll-input`（同步
  `agent-shell` delta 回主 spec）
