# Design

## Context

### 实测证据（2026-11-20 夜）

宿主 `[term-rate]`（`crates/agent-service/src/terminal/mod.rs` rate_log），
GUI debug 构建，pi-shell-rs `9df02dc`（含流式尾条行级 memo），会话 2138 条
消息 / 9MB（text 块最大 35KB、thinking 最大 44KB、工具卡最大 21KB）：

| 场景 | 输入→帧 | 帧→画 | render_cost | feed/s | 帧KB |
| --- | --- | --- | --- | --- | --- |
| 闲置打字（62×24） | avg 11–18ms | 7–13ms | 1.3–1.9ms | 4–5 | 0.2–2.0 |
| 流式中输入（62×24） | avg 38–46 max 65 | 5–11ms | 1.9–2.0ms | 19–44 | 1.5–2.5 |
| 流式中输入（125×36） | avg 85–106 max 167 | 7–14ms | 4.2–5.0ms | 9–25 | 4.8–9.5 |

（原始日志 `/tmp/term-rate.log` 为临时文件；数值已抄录，日志会被清理。）

### 对照实验（同一 pi-shell-rs 渲染链）

| 量测 | 数值 | 说明 |
| --- | --- | --- |
| standalone 真实流式回合（PTY 125×36）输入→帧输出 | **1.2ms** p50（max 1.3） | 流式进行中逐键探测 n=40 + 滚轮 n=10 |
| 纯流式帧间隔（standalone，无输入干扰） | p50=76ms（p90 193ms） | pi 事件推送节奏；宿主 feed/s=9–25 同源 |
| pi-shell-rs 流式整帧（bench_streaming_tail_under_budget） | 2.8ms avg / 3.3ms max（best-of-2） | 含行级 memo，40KB 互异代码块尾条 |
| 尾窗重建（35KB text + 44KB thinking + 21KB 工具卡尾条目） | 3.1ms/帧 | debug；MD/HL 缓存命中口径 |
| VirtualBackend 全屏 churn（125×36 全屏改写） | 2.6ms/帧 | CrosstermBackend diff→ANSI，debug |
| 宿主 vt100 feed（既有数据） | parser.process ≤0.5ms | `Docs/pending-decisions.md` |

**结论：渲染侧（pi-shell-rs draw → AnsiBuffer）输入到字节 ≤2ms；宿主测量的
38–106ms 全部消耗在字节产生之后的管线（drain tick / 推帧 / 泵 / feed）。**

### 管线解剖（现状代码路径）

```
输入: GUI key ──note_input──> SessionBus.send_key ──tell(KIND_INPUT_KEY)──>
      service actor on_input_key ──> SessionIo.inject_key(→ embed keys 通道)──>
      pi-shell-rs 事件循环(批量 drain, 每批一 draw) ──> AnsiBuffer
帧:   service tick (DRAIN_INTERVAL_MS=25, 自寻址 drain.tick, MissedTickBehavior::Delay,
      输入后补采 6 拍 8..48ms) ──take_ansi──> FrameBytes 推送 ──>
      GUI 泵 (50ms 主泵; 输入后 kick_input_drain 快泵窗 10×8ms, ServiceListener
      帧合并) ──> AgentTerminal::feed → vt100 parser.process ──note_feed──>
      cx.notify → render ──note_render──> 画出
```

闲置档快路径兑现（9–18ms）说明补采拍 + 快泵窗机制本身可用；**流式档未兑现**。

### echo 口径（现状）

`note_input`（面板发出输入）后第一个 `note_feed` 即计 echo（>300ms 记 miss）。
流式期帧源源不断，**下一个 feed 常是无关流式帧**——echo 样本被流式帧节拍
污染（且与 feed 节拍正相关：feed/s=44 时读数 38–46ms，feed/s=9–10 时读数
85–106ms，正比于帧间隔而非回显真实时延）。口径不修则修复无法验收。

## Goals / Non-Goals

**Goals:**

- 归因：以最小探针（t0→t1 + 快泵关窗归因，见仪器点）判定 H1/H3′/H4；
  H2 已经代码阅读排除（见假设段），不再为其打点；
- 口径：`[term-rate]` echo 只计回显帧，流式期读数可信；
- 修复：流式中输入→帧 avg ≤20ms（与闲置档同量级），滚动输入 ≤30ms；
- 闲置档不劣化（11–18ms 保持）；渲染侧零回退（pi-shell-rs 基线 1–3ms）。

**Non-Goals:**

- pi 事件推送节奏（p50=76ms/批 = 模型流速 + pi 内部批帧，宿主管线无关）；
- 流式帧率/滚动平滑度（渲染链已优化；帧率由 pi 推送节奏决定）；
- pi-shell-rs in-flight thinking/工具卡/bash 尾流行级 memo（pi-shell-rs 仓
  follow-up 候选，机制同源可复用；不在本 change）；
- 自发输出双相位（无输入触发的 ≤75ms 结构值）——本 change 只管**输入回显**；
  若调查证明回显路径与自发路径无法分离，再扩目标（见 Open Questions）。

## 归因分解（已定案：H5 引擎 batch-draw 合并）

**真因 H5 ✅**：`pi-shell-rs` `event_loop` 的 design D4「整批只 draw 一次
（合并高频 `tool_execution_update` 的重绘）」。`event.rs` 中输入 `keys` 与
RPC 流式读线程**共用同一 `tx/rx` channel** → `event_loop` 的 `recv_timeout`
\+ `try_recv` drain 把输入事件和流式 `UiEvent::Rpc` 收成一批 → `for ev in
batch` 整批处理、`if need_draw { draw }` 只画一次。流式期 batch 巨大（2k
长文含大量 Rpc）→ 单批处理 + draw 耗时长、帧大（5–21KB）→ 输入回显被
diff 进这个大帧、跟着整批 draw 出，延迟 = 等下一次大 draw。

排除链（实测 + 代码）：pi-shell-rs draw 单帧（1.2ms）、ANSI 生成（2.6ms）、
vt100 feed（≤0.5ms）、宿主 paint（render_cost 4–15ms，echo 止于 note_feed）
均不构成 38–617ms。

## 假设检验（H5 坐实，H1–H4 全排除）

| 假设 | 结论 | 证据 |
| --- | --- | --- |
| H1 补采拍被信封 FIFO 挤占 | ❌ | `[echo-path] t0→t1 avg=0.0ms`（闲置/流式皆 0）——邮箱不堵 |
| H2 帧合并吞并回显帧 | ❌ | `take_ansi`=`ansi.take()` 增量 drain、`pump` FIFO 逐帧 feed（代码） |
| H3′ 快泵窗退出条件歧义 | ❌ | 关窗归因全"新帧"（回显帧到了只是晚）；echo-seq 证回显被合并，非被快泵漏 |
| H4 口径污染（流式帧抢跑） | ❌ | 新口径（代际>基线首帧）≈旧口径（首 feed）读数一致（187 vs 183）→ 回显被合并，非被冒充 |
| **H5 引擎 batch-draw 合并（D4）** | **✅（部分）** | echo-seq 回显混大帧、随帧增大 270→617ms、断流 1.14s；`draw_batch_probe` 测试证一批 `[Rpc×30, Key]` 只 draw 1 次。输入优先 draw 后 62×24 回显变独立小帧 9ms（有效）；2k 仍慢 → 主因转 H6 |
| **H6 UI 构建 O(transcript)（布局缓存冷帧）** | **✅（2k 主因）** | `[draw-cost]` 分解：535ms 尖峰 = `ui=533.5ms`（diff=0.2ms/ansi=2.8ms 排除）= 布局缓存冷帧全量重排；cells=1 纯回显 draw 的 ui 也 8.5–9.3ms（闲置 1.6ms）→ 每帧 O(transcript) 重绘；回显 +10ms 可达 vs +223/+504ms 排在冷帧后 = 排队等 ui |

**逐帧铁证（`[echo-seq]`，2026-11-21 实测）**：闲置回显是独立小帧
`+9ms/40B`；流式期回显被并进大帧 `+270ms/5931B`、`+456ms/10249B`、
`+617ms/6297B`（帧越大越慢）；`baseline=170` 见 +71ms 后**断流 1.14s**才来
一串 3–21KB 大帧——回显等整批大 draw 的直接观感。

**探针盲区（口径局限，记入 Decisions）**：代际号 `advance()` 只在输入
dispatch 推进一次，输入后的**流式续帧与回显帧同代际**（均 > 基线），代际
无法区分。故 1.2 的 echo 口径实际计"输入后首个新代际帧"（常为合并大帧），
非纯回显帧；新旧口径读数一致（都测合并大帧到达）正源于此。严格分离回显需
方案 B（pi-shell-rs 帧头标记），随 H5 修复一并考虑。

## draw 耗时分解（定位 A，2026-11-22 `[draw-cost]` 实测）

`[draw-cost] total/ui/diff/ansi | cells/bytes`（ui=闭包内 view::render，
diff=ratatui buffer 比较余量，ansi=backend CSI 生成）：

| 场景 | total | ui | diff | ansi | cells | 判读 |
| --- | --- | --- | --- | --- | --- | --- |
| 闲档回显（cells=1） | 2.4ms | 1.6ms | 0.2ms | 0.7ms | 1 | 基线 |
| 流式期纯回显（cells=1） | 10.4ms | **8.9ms** | 0.2ms | 1.4ms | 1 | ui 涨 5.6 倍 = O(transcript) |
| 流式大帧 | 11–13ms | 8.5–9.3ms | 0.2ms | 2.8–4.2ms | 5k–12k | ui 主导 |
| **卡顿尖峰** | **536.5ms** | **533.5ms** | 0.2ms | 2.8ms | 7577 | **布局缓存冷帧**（≈4f7b6ef 冷帧 600ms） |

结论：**diff/ansi 排除**（各 ≤4ms）；主成本 = `view::render` 的布局/wrap
构建——暖帧 O(transcript) 基础成本 9ms，冷帧（条目增删→缓存失效→全量
重排）500ms+。流式期回显 draw 排在冷帧后（event_loop 单线程）→ 220–500ms。
与 pending-decisions"残余 = pi 侧重绘 O(transcript)"一致，方向：布局缓存
失效时增量重排 / 视口化 wrap 命中率提升（pi-shell-rs 渲染层）。

## 仪器点（已落地，消盲区后定案）

代际号原语（见 Decisions）落地后，原 t0..t5 六点全量打点不再需要。归因
已定案 H5，保留的探针如下（均为临时，随 2.3 拆除；`[draw-probe]`/
`draw_batch_probe` 在 pi-shell-rs 侧）：

| 探针 | 位置 | 作用 |
| --- | --- | --- |
| `[echo-path]` t0→t1 | `app.rs::send_key` → `service.rs::on_input_key` | H1 排队量（实测 0ms，排除） |
| `[echo-path]` 关窗归因 | `app.rs::kick_input_drain` `got=true` | 全旧帧关窗 = H3′（实测全"新帧"，排除） |
| `[echo-path]` 旧口径镜像 | `note_input` 开 / 首 feed 关 | 1.1 ±10ms 对照 vs 原始数值 |
| `[echo-seq]` 逐帧序列 | `echo_path::trace_*`（`note_input` 开/`note_feed` 落帧） | 消代际盲区：看回显小帧 vs 流式大帧排位/间隔 → **H5 铁证** |
| `[draw-probe]` | pi-shell-rs `event_loop` draw 处 | batch 组成 + draw 节奏 `batch=N input=I rpc=R gap=Gms draw_cost=Cms` |
| `draw_batch_probe` 测试 | pi-shell-rs `app::mod` | 确定性证 batch 合并（一批 `[Rpc×30, Key]` 只 draw 1 次） |

输出按秒汇总（对齐 `[term-rate]` 风格），同读 stderr。

## Decisions（定稿方向；H1 残差归因后可再调）

**顺序原则：尺子先行。** 代际号原语 + 口径修正（任务 1.2）本就是必做项、
不依赖归因结论，先落地再打最小探针——此时复测读数是真话，残差直接指向
H1。仍是先证后改（2.1/2.2 修复前必有归因数据），只是尺子不是 throwaway。

**代际号原语（三方共用支点，即方案 A）**：帧在 take_ansi/推送处带生产
代际（或全局帧序号），`note_input` 时记基线。同一原语服务三处：

| 用途 | 修什么 |
| --- | --- |
| `rate_log` 只计代际 > 基线的帧为 echo 样本 | H4 / 口径修正 |
| 快泵 break 改为"喂到代际 > 基线的帧才关窗" | H3′ / 快路径修复 |
| 探针关窗归因（仪器点归因行） | H3′ 直证 |

**口径修正（必做，已落地）**：采用方案 A（上表第一用途）。**口径局限
（H5 探针盲区）**：`advance()` 每输入推进一次，输入后流式帧与回显帧同
代际，故 echo 口径计"输入后首个新代际帧"（常为合并大帧），非纯回显帧。
严格分离需方案 B（pi-shell-rs 帧头携带回显标记——更准但动跨仓契约），随
H5 修复一并考虑；方案 C（流式期停用 echo 统计）作备选。误差界：帧被取走
时刻 ≥ 产生时刻，≤1 拍误差可接受。

**修复（H5，跨仓 pi-shell-rs）**：`event_loop` 让输入类事件（Key/Wheel/
Click/Pointer）**优先单独 draw**，不与流式 Rpc 合并到 batch 尾部——输入
回显不等整批大 draw。候选实现：
- 输入事件处理完立即 draw（打断 batch 的"整批一次 draw"，输入处多一次 draw
  但输入低频）；或
- batch 收集遇输入即断、输入优先处理 + draw，流式 Rpc 组内仍合并。
约束：流式帧率不劣化（feed/s 不降）、render 次数增幅可控（仅输入触发的额外
draw）。

**作废**：~~H3′ 快泵 break 改"喂到代际 > 基线的帧才关窗"~~（H3′ 已排除，
宿主快泵不是瓶颈，改了无用）；~~H1 mailbox 让位/补采拍改独立计时器~~
（H1 已排除）。

## Risks / Trade-offs

- [回显帧识别需要"帧产生时刻"可见] → 方案 A 用 take_ansi 时间戳近似
  （帧被取走的时刻 ≥ 产生时刻），存在 ≤1 拍误差；方案 B 精确但跨仓。
- [输入优先 draw（H5 修复）可能增加 render 次数] → 仅输入触发的额外 draw
  （输入低频），流式 Rpc 组内仍合并；约束流式 feed/s 不降、render 增幅可控。
- [口径修正未达预期（新旧读数一致）] → 探针盲区所致：输入后流式帧与回显帧
  同代际，口径实测合并大帧到达（非纯回显）。H5 修复后回显独立成小帧，口径
  才真正生效；验收以体感场景（流式中打字）人工复核 + 双记录过渡一轮。
- [多会话并发下 tick 自身耗时随会话数增长]（pending-decisions 已记 30+
  会话疑点）→ 打点顺带记录 tick 耗时与会话数，若相关则另立 follow-up。

## Open Questions

- 回显路径与自发输出路径能否完全分离？若不能，双相位 ≤75ms 是否也要纳入
  目标（即从"输入回显"扩到"全链路输入→帧"）？——倾向先按回显验收。
- 流式中滚轮（滚动回显）与打字是否同路径？（滚轮走 `KIND_INPUT_WHEEL` +
  同一快泵窗，预期同解；验收数值分开记录。）
- 62×24 → 125×36 读数差（38→106ms）是面板面积效应还是当轮内容效应
  （两窗口的流式内容不同）？打点复测时控制变量复核。
