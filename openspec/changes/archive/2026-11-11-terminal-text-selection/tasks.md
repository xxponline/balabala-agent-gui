# Tasks

## 1. 选区纯逻辑（状态与换算）

- [x] 1.1 `CellRange{anchor, focus}`（含规范化 start≤end、闭区间行/列）
  与像素↔cell 换算纯函数（走 `grid_bounds.origin` + `grid_metrics`，
  负值/超界双向 clamp——与点击坐标换算同源）；验证 pane 纯测
  `selection_range_normalizes_and_clamps`（反向拖选规范化、越界 clamp、
  CJK 宽字符落列）
- [x] 1.2 `selection_text(screen, range) -> String` 组装纯函数（哨兵与
  复制共用）：复用 vt100 官方 `contents_between`（实现揭示：其文档明言为
  clipboard selection 设计——软换行/续格/行尾空 cell 皆官方处理；闭区间
  靠尾列 +1 闭合，绕开单格半开坑）+ 按行 trim_end；验证 pane 纯测
  `selection_text_assembles_cells`（ASCII/CJK 混合、尾空格、单格非空、
  多行闭区间）

## 2. 事件接线与 overlay

- [x] 2.1 `TerminalPanel` 状态（`selection` / `press{down_pos, moved}`）+
  `.on_mouse_down/.on_mouse_move/.on_mouse_up_out` 接线（>3px 置 moved、
  拖动实时更新选区与快照、出界收尾复位）+ overlay 矩形层渲染
  （range→每行 rect，absolute 于 grid 上）；验证无头测试
  `drag_builds_selection_overlay`（`drag_to` 注入选区 → 选区状态与
  overlay 锚点元素可寻址，范围断言）
- [x] 2.2 手势分离：既有 `.on_click` 读 `press.moved`——true 只聚焦不发
  坐标、false 维持 `forward_click_coordinate`；验证无头测试
  `drag_selection_sends_no_click_coordinate`（拖选后 `click_forwards`
  不变、纯点击后 +1）且既有 `click_inside_grid_*` 两测试保持绿

## 3. 快照哨兵

- [x] 3.1 快照建立（选区建立/更新时存 `selection_text`）+ 帧路径比对
  （render 取 screen 时选区非空 → 实时文本 ≠ 快照 → 清选区；同则保留）；
  验证：纯测 `selection_snapshot_detects_drift`（文本变→清判定、文本同→
  保留判定）+ 无头 `selection_survives_unrelated_frames`（选区存在时
  无关帧到达不清）

## 4. Cmd+C 复制

- [x] 4.1 `TerminalCopy` action + `Terminal` context 下 `cmd-c` 绑定
  （`TerminalTab` 同款模式）；选区非空 → `selection_text` 实时组装 →
  `write_to_clipboard`，空则不动剪贴板；验证无头测试
  `cmd_c_copies_selection_to_clipboard`（设选区 → 注入 cmd-c →
  `read_from_clipboard` 断言内容与规则）与
  `cmd_c_without_selection_leaves_clipboard`（剪贴板不变）

## 5. 集成回归

- [x] 5.1 全量回归：本仓库 `cargo test` 全绿 + `cargo check --all-targets`
  0 warning；`openspec validate terminal-text-selection --strict` 通过
- [x] 5.2 headed 真机验收并记录：真实拖选手感与高亮、拖选不触发卡片、
  纯点击仍开卡片、Cmd+C 进系统剪贴板（含 CJK）、流式回答中选区被顶走
  即消失、选区外输出不误伤、resize/滚动清选区、无选区 Cmd+C 不动剪贴板

## Workflow follow-up

- commit 本仓库（feat 实现 + docs(openspec) 归档分两笔）
- review 后 `/opsx-archive terminal-text-selection`（同步 `agent-shell`
  delta 回主 spec）
