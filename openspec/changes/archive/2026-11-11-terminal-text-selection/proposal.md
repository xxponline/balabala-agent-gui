# Proposal

## Why

agent 控制台（终端面板）渲染 vt100 网格但没有任何文本选区：用户无法选中、
复制屏幕上的一段输出——摘录一段报错/链接/代码只能整屏肉眼抄。同时终端区域
的左键已有第二重身份（转发单元格坐标给 pi 命中卡片），引入拖选手势必须把
两种手势在契约层面分开，否则拖选结束会误触发 TUI 点击。

## What Changes

- **终端区域文本选区**：`TerminalPanel` 维护选区状态机——左键按下起锚、
  拖动跨 cell 扩展、松手定界；视觉用 overlay 矩形层（按行分段，
  `row×line_h / col×cell_w` 数学），不触碰既有 Segment 渲染；像素→cell
  换算复用 `grid_bounds` + `grid_metrics`，出界 clamp 到网格边缘
- **拖选手势与 TUI 点击分离**：gpui 的 click 组装**无移动阈值**（down/up
  同 hitbox 即 fire）→ 自维护 `moved` 标志（>3px 判定拖选）；拖选结束不
  转发单元格坐标，纯点击维持现状转发（**BREAKING** 行为细化：
  `Terminal area click coordinates forwarded` 需求加拖选手势例外）
- **快照哨兵清选区**：选区建立/拖动中快照选区位置文本；每次帧到达时
  实时重读比对——不同即清。覆盖一切漂移源（GUI 滚轮、翻页、pi 流式输出
  推屏、卡片展开、resize、切会话），且选区外的输出不误伤。快照只当
  staleness 哨兵，**复制内容实时读 vt100**（哨兵保证两者一致）
- **Cmd+C 复制**：`Terminal` key context 挂 action，选区规范化 → 逐行
  逐 cell 读 vt100（跳宽字符续格）→ 行尾去尾随空格、行间 `\n` →
  `write_to_clipboard`；无选区时不写剪贴板
- **明确不做**（Non-Goals）：双击选词/三击选行、Shift+方向键键盘选区、
  选区跟随滚动平移、选中暂停输出
- **spec `agent-shell` delta**：ADDED 选区、ADDED 哨兵清选区、ADDED
  Cmd+C 复制、MODIFIED 点击坐标转发（+拖选不发坐标场景）

## Capabilities

### New Capabilities

（无——控制台的鼠标/键盘交互是 `agent-shell`「终端面板转发用户输入」的
既有边界；选区不改变任何会话侧契约。）

### Modified Capabilities

- `agent-shell`: ADDED「终端区域文本选区」（拖动手势、overlay 视觉、
  出界 clamp）、ADDED「选区内容变化即清除」（快照哨兵、全源覆盖、
  选区外输出不误伤）、ADDED「Cmd+C 复制选区」（复制文本形态、无选区
  不写剪贴板）、MODIFIED「Terminal area click coordinates forwarded」
  （拖选手势不发坐标）

## Impact

- **修改**：`src/pane/mod.rs`（选区状态机 + overlay 渲染 + mouse 事件
  接线 + moved 手势判定 + 哨兵比对 + Cmd+C action + 复制组装）——
  单文件为主战场
- **依赖**：零新增（vt100 `screen.cell`/`Cell` API、gpui mouse 事件、
  `write_to_clipboard`、`grid_bounds`/`grid_metrics` 全部现成）
- **spec delta**：`agent-shell`（3 ADDED + 1 MODIFIED）
- **测试**：无头（gpui-kit `drag_to` 注入拖动、`read_from_clipboard`
  断言、哨兵帧变清选区、拖选后 `click_forwards` 不变）+ headed 真机
  验收（真实拖选手感、Cmd+C 到系统剪贴板、流式输出中选区稳定性）
