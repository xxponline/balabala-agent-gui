# Design

## Context

控制台渲染链（`src/pane/mod.rs`）：`build_grid(screen)` 把 vt100 屏折成
`Segment{text, fg, bg, cols}` 行/段，`terminal_grid` 渲染成真实元素
（`.id(("grid-row", r))` 每行、段 div 按 `cols×cell_w` 定宽）——选区视觉
有现成的坐标体系可挂。交互现状：`on_click` = 聚焦 + 发 TUI 单元格坐标；
`grid_bounds`（`on_children_prepainted` 存）+ `grid_metrics(cell_w, line_h)`
是上个 change 铺好的像素↔cell 换算原料。

关键机制事实（gpui-fast 源码实证）：
- **click 组装无移动阈值**：down 记入 `pending_mouse_down`，up 时只要
  hitbox 仍 hovered 就 fire `ClickEvent::Mouse{down, up}`——拖选结束必然
  触发 `on_click` → 若不区分会误发坐标给 pi
- `on_mouse_down/up/move` 均为 Bubble + `is_hovered` 门；库有
  `on_mouse_up_out` / mouse-exit 可收尾出界手势
- vt100：`screen.cell(r,c) -> Option<&Cell>`、`Cell::is_wide` /
  `is_wide_continuation`
- 剪贴板先例：`cx.write_to_clipboard(ClipboardItem::new_string)`（树右键）
- 键绑定：`Terminal` key context 已存在（`TerminalTab` 同款挂法）；
  `modifiers.platform` 已被 `handle_key_down` 挡在转发外（Cmd+C 不会漏给 pi）

动机与决策定案见 proposal.md Why / What Changes（探索轮已确认：Cmd+C、
不做双击三击与 Shift 键选、快照哨兵方案）。

## Goals / Non-Goals

**Goals:**
- 鼠标拖选 + overlay 高亮 + Cmd+C 复制，全链路无头可测
- 拖选手势与 TUI 点击坐标在契约层分离（pi 不被拖选污染）
- 选区漂移的**全源**正确性（哨兵覆盖 GUI 不可知的 pi 内部滚动）
- 零新增依赖；不改变 Segment 渲染与既有行宽契约测试

**Non-Goals:**
- 双击选词 / 三击选行 / Shift+方向键键盘选区（探索定案不做）
- 选区跟随内容平移（只覆盖 GUI 自己的滚动、盖不住 pi 推屏，成本高收益半）
- 选中暂停会话输出（原生终端也不做）
- 选区持久化（切会话/关签即失效，哨兵顺带处理）
- 右键菜单复制（Cmd+C 单途径起步）

## Decisions

### D1: 自绘状态机 + overlay 矩形层（否决 segment 上色 / 现成组件）

`TerminalPanel` 持 `selection: Option<CellRange{anchor, focus}>` +
`press: Option<Press{down_pos, moved}>`。视觉 = 一个 absolute overlay
层，从 range 算矩形：`y=row×line_h, x=col×cell_w, w=长度×cell_w`，跨行拆
每行一段 rect——纯数学，不触碰 `build_grid`/Segment。

- **备选（否）**：选中 segment 换 bg——选区边界常落在段中间，要拆
  Segment、每帧重算，复杂一档；视觉收益只是"高亮盖字 vs 换底色"
- **备选（否）**：Editor/文档组件承载——终端是 vt100 自绘网格非文档流；
  gpui 无原生文本选区 API（zed 系 UI 均自绘）

### D2: 手势判定——3px moved 阈值分离"拖选"与"TUI 点击"

`on_mouse_down` 记 `press{down_pos, moved:false}` 并清旧选区；
`on_mouse_move`（hover 内）当 `|Δ|>3px` 置 `moved=true` 并持续更新
`selection`；`on_click`（既有 handler）读 `press.moved`：
- false → 维持现状 `forward_click_coordinate`
- true → 只聚焦、**不发坐标**（spec: Drag selection delivers no coordinate）

出界收尾：`on_mouse_up_out`/mouse-exit 清 `press` 残留（up 不在元素内时
click 本就不 fire，但 `moved` 状态必须复位，否则下次点击误判）。
`on_mouse_move` 仅 hover 内 fire → focus 端天然 clamp 到网格边缘
（spec: clamps to the grid edge）；像素→cell 后再对 `grid_bounds.size`
与 screen 行列双重 clamp。

### D3: 快照哨兵——选区文本比对，事件点清理否决（探索定案）

选区建立/每次拖动更新时，从 vt100 实时读选区文本存
`selection_snapshot: String`；**新帧到达**（`poll_sessions` 返回 true 且
selection 非空）时重读比对：不同 → 清选区；相同 → 保留。

- **为何否决事件点清理**（只在滚轮/翻页/resize/切会话处手清）：漏掉
  最常见的漂移源——**pi 流式输出推屏**与**卡片展开推挤**（GUI 不可知），
  失败形态是无声复制错误文本
- **为何否决跟随平移**：只覆盖 GUI 自己发的滚动量，pi 自主滚动依然断；
  跨屏截断逻辑成本高
- **残漏分析**：仅"选区行文本滚动前后完全相同"（基本=空白/重复行）不被
  逮住 → 复制出空白，无害级
- **快照语义**：只当 staleness 哨兵；**复制内容永远实时读 vt100**——
  哨兵保证"能按 Cmd+C 时内容与所选一致"
- **不误伤**：选区外的流式输出不改选区坐标文本 → 比对通过 → 保留
  （spec: Selection survives unrelated frame updates）

### D4: 换算与 clamp 全走既有度量

像素→cell：`(pos − grid_bounds.origin) / (cell_w, line_h)`；负值或
超 `grid_bounds.size` → clamp 到边缘。cell→像素（overlay）逆运算。
与 4.1 点击坐标换算同源同尺（防两处漂移不一致）。

### D5: Cmd+C 挂 `Terminal` key context 的 action

`actions!(terminal, [...])` 增 `TerminalCopy`，`.key_bindings`/`bind_keys`
在 `Terminal` context 下绑 `cmd-c`（`TerminalTab` 同款模式）。处理：
selection 非空 → 组装文本 → `write_to_clipboard`；空 → 不动剪贴板。
`modifiers.platform` 已被 `handle_key_down` 拦截 → 不会同时漏给 pi。

### D6: 复制文本组装规则（spec 形态的实现）

复用 vt100 官方 `Screen::contents_between(sr, sc, er, ec+1)`——实现揭示：
其源码注释明言 "useful for determining the contents of a clipboard
selection"，软换行不插 `\n`、宽字符续格跳过、行尾空 cell 不输出皆为
官方处理。闭区间语义靠**尾列 +1** 闭合（库是半开边界；单格 `sc==ec`
原生返回空，+1 绕开）；库仍会保留程序写入的尾随空格 → 结果按 `\n`
split 后逐行 `trim_end` 再 join（spec: trailing whitespace per row）。

## Risks / Trade-offs

- **[拖选中移出网格后 focus 不再更新（hover 门）]** → v1 接受 clamp 到
  边缘（spec 已按此写）；要跟手需 window 级 move 监听，留扩展点
- **[空白/重复行的哨兵残漏]** → 无害级（复制出空白），探索已论证接受
- **[流式输出中选区被清的观感]** → 只在**选中区域自己被顶走**时清，
  选区外输出不误伤（spec 场景已锁）——正是想要的语义
- **[overlay 与网格错帧]** → overlay 与 grid 同 render 出（同一实体状态），
  同帧一致；哨兵清选区也在帧路径上
- **[`drag_to` 测试注入的坐标系]** → 实现首个任务即用无头拖动验证
  手势链路，若库注入不适用改用 down/move/up 事件序列直发

## Migration Plan

单 PR；无持久化/依赖/迁移。回滚 = revert。顺序：状态机与换算（纯逻辑
单测）→ 事件接线与 overlay → 手势分离 → 哨兵 → Cmd+C → 回归与验收。

## Open Questions

（无——复制途径、不做清单、哨兵方案均已在探索中定案。）
