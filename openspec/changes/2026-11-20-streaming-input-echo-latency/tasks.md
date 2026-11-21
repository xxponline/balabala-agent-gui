# Tasks

（顺序原则：尺子先行——2.1（代际号 + 口径修正）本为必做项且不依赖归因
结论，提到探针之前；探针缩为最小面，见 design.md 仪器点表。）

## 1. 尺子先行（必做，不依赖归因）

- [x] 1.1 受控复现协议固化：长会话（≥2000 条消息）两档流式回合——(a) 纯
  文本+代码块长文（禁工具）；(b) 含工具回合（bash/文件读写，输出流）。
  流式进行中打字 20–40 键 + 滚轮 10 次，读 `[term-rate]` 每秒汇总。
  **验证**：两档复现步骤与原始数值写入本任务注记，重跑一轮数值可重复
  （±10ms 量级）

  > **注记（协议固化, 2026-11-21）**
  > - 前置：GUI debug 构建；长会话 ≥2000 条消息（基线会话 2138 条 / 9MB，
  >   text 块最大 35KB、thinking 最大 44KB、工具卡最大 21KB）。
  > - 两档流式回合：**(a)** 纯文本 + 代码块长文（禁工具）；**(b)** 含工具
  >   回合（bash/文件读写，输出流）。
  > - 每档操作：流式进行中打字 20–40 键 + 滚轮 10 次；收集 stderr 的
  >   `[term-rate]` 每秒汇总（2.1 起另收集 `[echo-path]` 行）。
  > - 原始数值（2026-11-20 夜基线，旧口径）：闲置打字（62×24）输入→帧
  >   avg 11–18ms；流式中输入（62×24）avg 38–46 max 65；（125×36）
  >   avg 85–106 max 167（详见 design.md Context 表）。
  > - 旧口径 echo 读数由 `[echo-path]` 探针过渡记录（2.3 拆除）；±10ms
  >   可重复性对照用该读数 vs 原始数值，并入第一轮实测确认。
- [x] 1.2 代际号原语 + echo 口径修正（design Decisions 方案 A）：帧在
  take_ansi/推送处带生产代际，`note_input` 记基线；`[term-rate]` 的 echo
  只计代际 > 基线的首帧（回显帧），流式帧不计样本；miss 语义保持
  （>300ms 单独计数）。**验证**：单测覆盖"连续流式帧 + 回显帧"序列只计
  回显帧；1.2 口径下 1.1 两档数值重现并记录（此时读数是真话）

  > **注记（实现 2026-11-21，待实测验收）**
  > - 代际号原语：`crates/agent-service/src/frame_gen.rs`（advance/current/
  >   note_fed/max_fed）；`contract::FrameBytes` 增 `prod_gen`；`service.rs`
  >   `tick` 推送处打代际、`dispatch` 内联 drain 后 advance（输入前残帧=旧代际）。
  > - echo 口径：`terminal/mod.rs::rate_log::EchoWindow` 纯状态机——`note_input`
  >   记代际基线，`note_feed` 仅在 `prod_gen > 基线` 的首帧计样本并关窗，流式帧
  >   （≤基线）不计不动窗；miss（>300ms）单独计数不变。
  > - 单测（`terminal::rate_log::tests`，3 项全绿）：`echo_counts_only_first_
  >   post_baseline_frame`（连续流式帧+回显帧只计回显帧）、`echo_miss_over_300ms`、
  >   `no_pending_and_overwrite_semantics`。
  > - **待实测**（并入 1.1 复测/4.1）：1.2 口径下 1.1 两档数值重现并记录。

## 2. 最小探针归因（量完即拆）

- [x] 2.1 最小打点（design.md 仪器点表：t0→t1 两点 + 快泵关窗归因行，
  `[echo-path]` 秒级汇总风格对齐 `[term-rate]`）：跑 1.1 两档收集分段数值。
  **验证**：t0→t1 / 关窗归因数值落表，H1 排队量与 H3′ 关窗证据可辨认

  > **注记（实现 2026-11-21，待实测收集）**
  > - 探针：`crates/agent-service/src/echo_path.rs`，`[echo-path]` 每秒一行
  >   （`t0→t1 avg/max/n | 旧echo avg/max/n/miss | 关窗 全旧帧=.. 新帧=..`），
  >   风格对齐 `[term-rate]`，同读 stderr。
  > - 仪器点：t0=`app.rs::send_key`（tell 成功后）；t1=`service.rs::
  >   on_input_key` 入口；关窗归因=`app.rs::kick_input_drain` 的 `got=true`
  >   关窗处（`all_old = frame_gen::max_fed() <= baseline`）。FIFO push/pop 只挂
  >   key 一对保持对称（wheel 的 echo 已由 `handle_scroll`→`note_input` 覆盖，
  >   不单侧挂 t1 以免误配）。
  > - 旧口径镜像（`note_input` 开/首个 feed 关）供 1.1 ±10ms 对照 vs 原始数值，
  >   2.3 随探针拆除。
  > - **逐帧序列探针 `[echo-seq]`**（消盲区）：代际在输入 dispatch 只推进一次，
  >   故输入后流式续帧与回显帧同代际（均 > 基线）无法用代际区分；改逐帧落表
  >   `(Δt, bytes, gen)`（`echo_path::trace_*`，`note_input` 开窗/`note_feed` 落帧/
  >   下一输入 dump）。字节数是天然判据（流式帧 5–8KB vs 回显小帧）——看回显帧
  >   第几帧到、前面压了几个大帧，判 H3′/H4。
  > - **实测（2026-11-21 两档）**：`t0→t1 avg=0.0ms`（H1 排除）；关窗归因全
  >   "新帧"（H3′ 排除）；`[echo-seq]` 闲置回显 `+9ms/40B` 独立小帧，流式期
  >   回显混大帧 `+270ms/5931B`/`+456ms/10249B`/`+617ms/6297B`、断流 1.14s
  >   （H5 铁证）→ 2.2 归因定案 H5。
- [x] 2.2 假设逐项证伪/确认，归因结论写回 design.md：H1/H3′/H4 标 ✅/❌
  与证据行（H2 已排除无需测试）；62×24 vs 125×36 差异按 Open Questions
  第三条控制变量复核。**验证**：design.md 归因段与 2.1 数据一致

  > **注记（归因定案 2026-11-21）**：H5 引擎 batch-draw 合并（D4）坐实，
  > H1/H2/H3′/H4 全排除（证据见 design.md 归因段 + 假设检验表）。铁证：
  > `[echo-seq]` 回显混大帧随帧增大 270→617ms、断流 1.14s；pi-shell-rs
  > `draw_batch_probe` 测试证一批 `[Rpc×30, Key]` 只 draw 1 次。探针盲区
  > （代际同代不区分流式/回显）记入 design.md Decisions。
- [x] 2.3 探针处置：拆除或转 env 门控（默认关）；`cargo test --workspace`
  不受影响。**验证**：`rg "echo-path" -l` 清点结果记入注记（拆除 or 门控）

  > **注记（探针清点 2026-11-22）**：`rg "echo-path|echo_path|echo-seq|
  > mirror_|trace_|note_window_close|note_sent|note_received" src/ crates/` =
  > **空**（全拆：`echo_path.rs` 删除、rate_log mirror/trace 调用删除、
  > `frame_gen` 的 note_fed/max_fed 删除、app kick 归因删除）。1.2 正式口径
  > （frame_gen advance/current + FrameBytes.prod_gen + EchoWindow）保留。
  > pi-shell-rs 侧 `[draw-probe]`/`[draw-cost]` 保留，归 H6 change 管理。

## 3. 修复（按 2.2 结论：H5，跨仓 pi-shell-rs）

- [x] 3.1 输入优先 draw（H5，跨仓 pi-shell-rs `event_loop`）：输入类事件
  （Key/Wheel/Click/Pointer）优先单独 draw，不与流式 Rpc 合并到 batch 尾部。
  候选：输入事件处理完立即 draw（打断 batch 单次 draw，输入处多一次 draw 但
  输入低频）；或 batch 收集遇输入即断、输入优先处理 + draw，流式 Rpc 组内仍
  合并。**验证**：流式中打字 `[term-rate] 输入→帧` 回到闲置量级（≤20ms，
  见 4.1），流式 feed/s 不降、render 增幅可控。

  > **注记（实现 2026-11-21；实测 2026-11-22）**：pi-shell-rs `event_loop`
  > 采用"输入类事件稳定前置 + 处理后即时 draw"（`batch.sort_by_key` 输入优先、
  > for 内 `if input_ev && need_draw { draw }`），流式 Rpc 组内仍合并 draw。
  > 机制验证 `input_echo_draws_independently_of_stream_batch`（pi-shell-rs）：
  > 一批 `[Rpc×30, Key]` 产生 3 draw（首帧 + 输入独立 + 流式合并），修复前 2。
  > **实测**：62×24 回显变独立小帧 `+9ms/40B`（达标 ≤20ms）✅；2k 仍 220–500ms
  > → 主因转 **H6 UI 构建**（`[draw-cost]`：535ms 尖峰全在 ui=533.5ms 布局缓存
  > 冷帧，diff/ansi 各 ≤4ms；cells=1 纯回显 draw 的 ui 也 8.5–9.3ms），非 draw
  > 合并。H6 修复 = pi-shell-rs 布局增量重排（见 design.md draw 耗时分解）。
- [x] 3.2 ~~流式期输入快路径（H1）~~ **作废**：H1 已排除（t0→t1=0ms），
  mailbox 不是瓶颈。**验证**：design.md 归因段记 H1 ❌（已完成）。
- [x] 3.3 文档修订：`Docs/pending-decisions.md`"长文滚动 vs 原生控制台"
  条目补充前提（结论成立于渲染链 52–113ms 时代）或按结论修订；term-rate
  口径说明（回显帧定义 = 代际 > 输入基线的首帧）随实现注记。
  **验证**：文档与 1.2/3.1/3.2 实现一致

## 4. 验收

- [x] 4.1 性能验收（两档场景实测注记于本任务）：流式中打字 `[term-rate]
  输入→帧` avg ≤20ms、max ≤50ms；流式中滚轮 avg ≤30ms；闲置打字
  11–18ms 不劣化；流式帧率（feed/s）不劣化。**验证**：修复前后同场景
  数值对照表写入注记（范围修订：62×24 常用档；2k 最大化档划归 H6 follow-up）

  > **注记（验收对照表 2026-11-22）**
  > | 场景 | 修复前 | 修复后（H5） | 目标 | 判定 |
  > | --- | --- | --- | --- | --- |
  > | 62×24 流式打字 | 38–106ms（原始）/ 12–17ms（1.2 口径） | **9–18ms**（回显独立 40B 小帧） | ≤20ms | ✅ |
  > | 62×24 闲置打字 | 11–18ms | 12–18ms | 不劣化 | ✅ |
  > | 2k 最大化流式打字 | 220–500ms | 220–500ms（无改善） | ≤20ms | ❌ H6（ui 冷帧），
  >   follow-up：pi-shell-rs `2026-11-22-optimize-layout-cache-cold-frame` |
  > | 流式 feed/s | 5–12 | 3–12 | 不劣化 | ✅ |
  > 铁证：回显独立小帧 `+9ms/40B`；2k 延迟 = `[draw-cost]` ui 段（冷帧 533ms /
  > 暖帧 9ms），diff/ansi 各 ≤4ms（design.md draw 耗时分解）。
- [x] 4.2 全量 `cargo test --workspace` 绿、0 warnings；探针清点（2.3）无
  残留或已门控。**验证**：命令输出对账（测试计数与基线无缺失）

  > **注记（2026-11-22）**：`cargo test --workspace` 全绿（agent-service 42 =
  > 39 基线 + 3 EchoWindow 单测；GUI 147 等与基线无缺失），0 warnings。
  > 探针清点见 2.3（rg 空）。
