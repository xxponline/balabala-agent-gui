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

- 归因：把 38–106ms 分解到具体段（t0..t5，见下），H1–H4 逐项证伪；
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

## 归因分解（现状认知，待 1.2 打点证实）

已排除：pi-shell-rs draw（1.2ms）、ANSI 生成（2.6ms）、vt100 feed（≤0.5ms）、
宿主 paint（render_cost 4–5ms，且不计入 echo——echo 止于 note_feed）。
剩余嫌疑集中在：**service actor 的信封调度** 与 **GUI 泵的帧合并/快泵窗**。

## 假设（H1–H4，调查逐项证伪）

- **H1 补采拍被信封 FIFO 挤占**：drain.tick 是自寻址信封，与 KIND_INPUT_KEY
  同一 mailbox 顺序处理；流式期 tick 频度（25ms）+ 会话数多时 tick 自身耗时
  变长（MissedTickBehavior::Delay 顺延），输入信封/补采拍排队 → 输入注入
  延迟。看点：`on_input_key` 入口时刻 vs `note_input` 的差值（t0→t1）。
- **H2 帧合并吞并回显帧**：流式期 FrameBytes 密集，ServiceListener 合并语义
  把回显帧与相邻流式帧合并到下一拍才投递（"帧合并红利"反向作用）。
  看点：FrameBytes 推送（t3）→ 泵收到（t4）→ feed（t5）的间隔。
- **H3 快泵窗在流式期失效**：`kick_input_drain` 的 10×8ms 快泵与流式帧 notify
  竞争，或快泵只对"帧到达"起效而流式期帧已在途（set_view 快路径/泵节流）。
  看点：输入后 0–80ms 内 pump 次数（流式 vs 闲置对照）。
- **H4 口径污染为主**：真实回显只比闲置档多一个 drain tick 相位（~25ms），
  读数的 85–106ms 主要是把流式帧当回显帧计。看点：口径修正后的分布。

## 仪器点（临时探针；量完即拆或 env 门控保留）

| 时刻 | 位置 | 含义 |
| --- | --- | --- |
| t0 | `src/app.rs::send_key`（`note_input` 同点） | 输入发出 |
| t1 | `crates/agent-service/src/service.rs::on_input_key` 入口 | 服务收到输入 |
| t2 | `SessionIo::inject_key`（embed keys 通道 send） | 注入引擎 |
| t3 | `service.rs::tick` 推 FrameBytes 处（带帧字节数） | 帧推送 |
| t4 | GUI 泵收到 frame_bytes（`service_layer` pump 分流处） | 帧到达 UI |
| t5 | `terminal/mod.rs::feed`（`note_feed` 同点） | 喂入 parser |

输出按秒汇总（对齐 `[term-rate]` 风格，如 `[echo-path] t0→t1 avg/max …`），
与 `[term-rate]` 同读 stderr。pi-shell-rs 侧无需打点（已证 1.2ms）。

## Decisions（待 1.3 归因后定稿；以下为选项）

**口径修正（2.1，独立于归因结论必做）**：

- 方案 A（推荐）：echo 只计**输入后首个产生于输入之后的帧**——实现上给输入
  打代际号（`note_input` 时记 `input_gen++`），帧到达时若其字节产生于
  `note_input` 之后（take_ansi 时间戳或帧序号）才计 echo 样本；
- 方案 B：pi-shell-rs 在输入触发的 draw 上携带标记（如 AnsiBuffer 帧头注释
  或事件回调），宿主据此识别回显帧——更准但要动 pi-shell-rs 契约；
- 方案 C：流式期（feed 持续到达）直接停用 echo 统计，另设"回显专项"场景
  （流式中静默敲键并观察首帧）——测量口径与验收口径分离。

**快路径修复（2.2，按 H1–H3 结论择行）**：

- H1 成立 → 输入信封优先于 drain.tick（mailbox 让位/合并 tick），或补采拍
  改为独立计时器不经信箱；
- H2 成立 → 回显帧不参与合并（帧带代际号，输入后首帧直达 feed）；
- H3 成立 → 快泵窗与流式帧到达解耦（帧到即喂 feed，泵只控制 render 次数）。

## Risks / Trade-offs

- [回显帧识别需要"帧产生时刻"可见] → 方案 A 用 take_ansi 时间戳近似
  （帧被取走的时刻 ≥ 产生时刻），存在 ≤1 拍误差；方案 B 精确但跨仓。
- [输入优先/不合并可能加重流式期 UI 线程压力] → 上限约束：快泵窗总时长
  不变（80ms），仅调整投递顺序；render 次数不增加。
- [口径修正会让"流式期输入→帧"读数骤降，掩盖真实体感] → 验收以体感场景
  （流式中打字）人工复核 + 修正口径双记录过渡一轮。
- [多会话并发下 tick 自身耗时随会话数增长]（pending-decisions 已记 30+
  会话疑点）→ 打点顺带记录 tick 耗时与会话数，若相关则另立 follow-up。

## Open Questions

- 回显路径与自发输出路径能否完全分离？若不能，双相位 ≤75ms 是否也要纳入
  目标（即从"输入回显"扩到"全链路输入→帧"）？——倾向先按回显验收。
- 流式中滚轮（滚动回显）与打字是否同路径？（滚轮走 `KIND_INPUT_WHEEL` +
  同一快泵窗，预期同解；验收数值分开记录。）
- 62×24 → 125×36 读数差（38→106ms）是面板面积效应还是当轮内容效应
  （两窗口的流式内容不同）？打点复测时控制变量复核。
