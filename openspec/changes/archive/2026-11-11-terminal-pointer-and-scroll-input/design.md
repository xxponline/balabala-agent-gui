# Design

## Context

pi-shell-rs（`ref/` path 依赖，已可修改）三个归档 change 提供了完整能力：
`UiEvent::Wheel(i16)`（有符号行数）、`UiEvent::Click{x,y}`（0-based 单元格）、
`UiEvent::Key`（`KeyIntent::ScrollPage` 消费 PageUp/PageDown）。嵌入通道
`EventSources.keys: Sender<UiEvent>` 存在，但 `EmbeddedApp` 只暴露
`inject_key`/`inject_text`——**缺门面**。GUI 侧 `keystroke_to_keyevents`
丢弃 pageup/pagedown，`TerminalPanel` 无 `.on_scroll`、点击只做聚焦。

机制已验证（gpui-fast 源码）：
- 滚轮走 `dispatch_mouse_event`，bubble 阶段 `mouse_listeners.iter().rev()`
  （**内层先于外层**），`stop_propagation` 即停 → 中槽 `tab-content` 的
  `overflow_y_scroll` 容器在更外层，截不到
- `on_scroll` 执行条件 = `Bubble && hitbox.should_handle_scroll`（命中链判定，
  hover 语义，与键盘焦点无关）
- gpui 键名 `"pageup"`/`"pagedown"` 存在（keystroke.rs:465）
- 坐标链路：embed TUI 帧 → vt100 → GUI 网格同一数据同一 resize 同步，
  `grid_metrics(window) -> (cell_w, line_h)` 现成

动机见 proposal.md Why。

## Goals / Non-Goals

**Goals:**
- PageUp/PageDown、滚轮（行数）、左键点击（单元坐标）三路输入端到端送达
  active session，与 standalone pi-shell 手感语义一致（同坐标系、同 Alt 倍率）
- 零 pi 逻辑改动（纯门面加法）；零新依赖；无头测试可覆盖三路转发
- 焦点/滚轮语义与既有焦点模型自洽（键盘跟 active tab，滚轮跟 hover）

**Non-Goals:**
- **v1 不做速度加速**：`WheelAccel` 输入模型是"一事件 = 一 notch（dir）"，
  与触控板连续像素流错位（惯性小事件全算 notch → 疯狂加速）。macOS 原生
  行粒度滚动已够顺；`WheelAccel` 留作 v2 现成材料（届时 pi 加
  `next_scaled(dir, base, now)` 或按"凑满行的事件"打点）
- 不做终端 mouse tracking 协议（pi-shell TUI 是自身坐标命中，非协议转发）
- 不在 GUI 解释坐标语义（卡片命中表归 pi-shell，其 spec 已覆盖）
- 不接右键/拖拽/移动（pi 侧 `add-tool-card-click` 边界同样只转左键 Down）
- viewer 签的滚轮/点击（editor 自己处理，与本链路无关）

## Decisions

### D1: 换算在 GUI，pi 只加两个 send 门面

`EmbeddedApp::inject_wheel(lines: i16)` / `inject_click(x: u16, y: u16)`
各一行 send；`src/app/mod.rs` 的 `mod scroll` 改 `pub mod scroll` 让 GUI 引
`ALT_WHEEL_MULTIPLIER`（避免字面 5 与 pi 漂移）。

- **为何换算不在 pi**：pi 不知道行高——`line_h`（21px）是 GUI 字体度量
  （`grid_metrics`），pi 侧只有"行"语义。像素→行必须发生在持有度量的一侧
- **备选（否）**：pi 加 `inject_wheel_pixels`——pi 要么硬编码行高要么再收
  行高参数，接口更丑且逻辑分叉
- **备选（否）**：GUI 泛化 `inject(UiEvent)` 透传——门面太宽，把 `UiEvent`
  内部枚举变成宿主契约，pi 侧枚举演进即破坏宿主

### D2: 像素余数累积 + resize 清余数

`TerminalPanel` 持 `wheel_remainder: f32`。`ScrollDelta::Pixels{y}` →
`remainder += y / line_h`；`Lines{l}` → 直接与 remainder 相加取整；
`trunc()` 出整数行即发送并扣减。`line_h` 变化（`propagate_grid_size`
回来的度量变了）→ 清 remainder（spec scenario: Remainder resets）。
一次事件凑出多行（惯性一坨 delta）→ 一次 `send_wheel(n)` 带总行数
（`Wheel(i16)` 本就多行语义）。

### D3: Alt 倍率在 GUI 乘

`ScrollEvent.modifiers.alt` × `ALT_WHEEL_MULTIPLIER`（pi pub 常量）。
换算完、发之前乘。standalone 的同款行为在
`ui_event_from_crossterm` 转换处乘——两侧各自入口处乘，语义一致。

### D4: grid origin 由 `on_children_prepainted` 直存面板

`.on_click`/`.on_scroll` 拿到的是**窗口坐标**；单元换算需要
`(窗口坐标 − grid_origin) ÷ (cell_w, line_h)`。grid origin 在 prepaint
阶段才有：`#terminal-grid` 挂 `Div::on_children_prepainted`（裸闭包，
render 中不得 update 实体 → `Rc<Cell<Point<Pixels>>>` 存在
`TerminalPanel`，gpui-component 库测试同款手法），点击 handler 读字段
换算；`col/row` 越界（负值或 ≥ cols/rows）→ 不发（spec: Click outside
the grid delivers nothing）。

- **实现揭示修订（2026-11-11）**：原案 canvas prepare bounds +
  `cx.defer` 入 TerminalApp——实测 canvas bounds 的 y 是**内容坐标**
  （x 窗口系正确、y = 网格顶 + 内容行高，与 observed origin 差
  rows×line_h），不可作换算减项；改用 child prepaint bounds（与观测
  同源，测试 `grid_origin_tracks_terminal_grid_bounds` 断言等值）
- **备选（否）**：布局常量自算（tab bar + status bar 高度相加）——
  跨顶栏形态/字号变更脆弱
- **备选（否）**：运行时 hit_test/rendered_frame 查元素 bounds——
  gpui 均为 `pub(crate)`，不可用

### D5: `AgentEntity` 加两个具体方法

`send_wheel(lines: i16)` / `send_click(x: u16, y: u16)`，与
`send_key`/`send_text` 并排；`PiAgentShell` 直通 `embed.inject_*`；
Fake/Counting 测试假实现跟改（记录计数供断言）。

- **备选（否）**：`send_ui_event(UiEvent)`——把 pi 的类型漏进 trait 边界，
  破坏 `agent_entities` 的中立性（该模块刻意只依赖 crossterm 类型）
- `TerminalApp::send_wheel/send_click` 同 `send_key` 模式：active session
  分发 + `wheel_forwards`/`click_forwards` 诊断计数（`tab_forwards` 先例）

### D6: 焦点不参与、hover 决定归属

滚轮/点击 handler 挂 `#terminal-pane` 元素树（含 grid）：
- viewer active → console 不渲染 → handler 不存在 → 天然无事件（spec
  scenario 已断言）；无需焦点检查
- 点击仍执行现有 `focus.focus(window, cx)`（一步聚焦语义不变），随后发坐标
- 滚轮 `cx.stop_propagation()`：内层先冒泡（机制已证），外层容器收不到

### D7: 输入触发的快速 drain（卡顿修复, 2026-11-10 headed 验收追加）

滚动卡顿主因 = 帧获取的 50ms 节流主循环：pi 重绘写入 buffer 后，
要等下一次 tick 才被 `poll_sessions` 取走——滚动连续流下 0~50ms 相位
抖动直接可见（打字同链路但孤立事件不敏感）。方案 A（用户选定）：

- `poll_sessions` 改为返回 "是否消费到新帧"（原内部 `changed` 语义提升）
- 新增 `_input_drain_task: Option<Task<()>>` burst：输入后 8ms×4 拍
  快速 drain，**取到帧即停**（pi 每事件通常 1 帧；4 拍 ≈32ms 覆盖
  pi 线程调度+重绘时延），新输入重置（drop 旧 task，滚动风暴不叠加）
- kick 调用点：`send_key` / `send_text` / `send_wheel` / `send_click` /
  `propagate_grid_size`（全是“pi 产生新帧的用户触发源”）；空闲仍靠
  50ms 主循环（不烧 CPU，不改 RPC 流式输出的既有帧率）
- **备选（否）**：主 tick 收到 16ms——空闲也 60 次/秒轮询，CPU/电池代价
- **备选（否）**：pi 完成回调通知——要改 pi 的 AnsiBuffer 为条件变量，
  跨仓扩范围；burst 采样已覆盖该时延
- 无头可测性：测试的 app 不启动主循环（`start_update_loop` 只在
  `main.rs` 调）→ 输入后帧只能来自 burst，贡献可隔离断言

（spec 层不加延迟类需求——帧调度属实现质量，验收由 headed 手感 +
无头 burst 单测覆盖。）

## Risks / Trade-offs

- **[触控板惯性无加速，长历史翻页偏慢]** → v1 接受（Non-Goal）；PageUp/
  PageDown 是整页手段，快捷路径已在；v2 接 `WheelAccel`（材料现成）
- **[dock `tab-content` 抢滚轮的预判靠源码推理]** → 实现首个任务即在真机
  验证（挂 handler 后滚一次看 session 计数），推理错则改挂更内层/捕获
  阶段——机制文档给了 `stop_propagation` handler 的确定性做法
- **[origin defer 一帧延迟]** → 点击总发生在渲染之后，字段已是当前帧值；
  极端首帧点击（origin 全零）→ 越界判定兜底不发
- **[pi-shell-rs 副本改动与本仓库不同仓提交]** → 两仓各自 commit
  （ref/ 是 path 依赖，修改立即生效；注意 ref 副本有自己的 git 历史）
- **[滚动行数与 TUI 重绘帧率耦合]** → pi `handle_wheel` 返回 bool 驱动重绘，
  `take_ansi` 轮询（`poll` 50ms 节流）已有，无需新机制

## Migration Plan

单 PR（本仓库）+ 单 commit（ref/pi-shell-rs）；无数据/布局/持久化变化。
回滚 = revert 两处；pi 侧门面是纯加法，GUI revert 后不影响 pi 独立运行。

## Open Questions

（无——换算归属、加速取舍、Alt 倍率、spec 边界均已在探索中定案，见
proposal What Changes 与上文 Decisions。）
