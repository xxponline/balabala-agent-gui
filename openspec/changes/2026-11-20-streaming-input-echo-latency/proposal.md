# Proposal

## Why

`[term-rate]` 实测（2026-11-20 夜，pi-shell-rs `9df02dc`，GUI debug 125×36 面板，
会话 2138 条消息 / 9MB）：闲置打字 `输入→帧` **11–18ms**，**流式进行中**
输入→帧 **38–106ms（max 167ms）**——体感"打字仍有延迟"。

对照实验已证明**渲染链无罪**（同一 pi-shell-rs 引擎、同一渲染路径）：

| 量测 | 数值 | 结论 |
| --- | --- | --- |
| standalone 真实流式回合（PTY 125×36）输入→帧输出 | **1.2ms**（p50，n=40 键 + 10 滚轮） | pi-shell-rs 收键即画 |
| pi-shell-rs 流式整帧基准（含行级 memo） | 2.8ms（best-of-2 max 3.3） | 渲染已无 O(尾条) |
| 尾窗重建（35KB text + 44KB thinking + 21KB 工具卡） | 3.1ms/帧 | 大条目无放大 |
| VirtualBackend 全屏 churn（debug） | 2.6ms/帧 | ANSI 生成无放大 |
| 纯流式帧间隔（standalone） | p50=76ms | pi 事件推送节奏，与宿主 feed/s=9–25 同源 |

standalone 同链 1.2ms vs 宿主 38–106ms 的差值**全部落在宿主管线**（service
drain 25ms tick + GUI 泵/帧合并）。`Docs/pending-decisions.md` 现有结论
"管线侧无可挖余量、双相位结构上 ≤75ms"是在**渲染链 52–113ms 的旧前提**下
得出的；渲染链修到 1–3ms 后，管线相位成为唯一长板——流式期输入快路径
（服务补采 6×8ms 拍 + GUI 快泵 10×8ms 窗）没有像闲置档那样兑现。

另有一半问题在**口径**：`rate_log` 的 echo = "输入注入 → 下一个 `note_feed`"，
流式期下一个 feed 常是无关的流式帧（非回显帧）→ 数值被流式帧节拍污染/偏置，
真实回显可能明显小于读数。口径不修，修复效果也无法验收。

## What Changes

- **调查先行（先证后改）**：分段打点定位 38–106ms 落在哪一段（输入注入 →
  服务处理 → 帧推送 → 泵 → feed），对 design.md 的 H1–H4 假设逐项证伪；
  探针量完即拆（或 env 门控保留）。
- **echo 口径修正**：`[term-rate]` 只计**回显帧**（输入效果所在的首帧），
  流式帧不再计入 echo 样本；口径修正后同场景复测。
- **流式期输入快路径修复**（按归因结论择行）：使流式中输入→帧与闲置档
  同量级（avg ≤20ms）。
- **文档修订**：`Docs/pending-decisions.md` "管线无可挖余量"结论补充前提
  或修订；term-rate 口径说明随实现注记。
- **不做**：pi-shell-rs 渲染侧（已完成，见 pi-shell-rs 仓
  `optimize-streaming-tail-rendering`，9df02dc）；pi 事件推送节奏
  （p50=76ms/批，模型流速决定）；in-flight thinking/工具卡/bash 尾流的
  行级 memo（pi-shell-rs follow-up 候选，与本 change 无依赖）。

## Capabilities

### New Capabilities

（无。）

### Modified Capabilities

（无——性能与调试可观测性修复，不改用户可见行为；`.openspec.yaml` 已标
`skip_specs: true`。若调查结论涉及可观察行为变化（如输入合并/丢键语义），
实现前补 capability delta 并转 `skip_specs: false`。）

## Impact

- **代码**：`crates/agent-service/src/service.rs`（drain 循环 / 补采拍 /
  `on_input_key` / 信封优先级）、`src/app.rs`（`kick_input_drain` / 泵合并）、
  `crates/agent-service/src/terminal/mod.rs`（`rate_log` 口径、`feed` 合并语义）。
- **文档**：`Docs/pending-decisions.md`（"管线无可挖余量"条目修订）；
  归因数据回填本 change design.md。
- **依赖**：pi-shell-rs `9df02dc+`（渲染链基线；宿主 `Cargo.lock` 已更新）。
- **测试**：`rate_log` 口径单测；1.1 复现协议的实测数值记入任务注记
  （性能手工验收，与 agent-service 既有注记风格一致）。
