# Proposal

## Why

H6（pi-shell-rs 布局缓存冷帧）已由 pi-shell-rs change
`2026-11-22-optimize-layout-cache-cold-frame` 修复（失效路径增量重排 + 暖帧
按条目缓存命中），并经本仓 release 档集成复测达标：

- 2k 最大化流式打字 `[term-rate] 输入→帧` avg=11–18ms max=26ms（对照
  220–500ms）✅；`[draw-cost]` ui 回显/流式大帧 ≈0.9ms、无冷帧尖峰 ✅
- 附带定案：此前"卡顿感"主因是本仓以 debug 档（`cargo run`）运行——paint/
  工具卡渲染慢 3–6 倍（同场景 输入→帧 144–248ms）；性能验收与日常使用应
  跑 `cargo run --release`

遗留收口事项（本 change 承载）：

1. 2026-11-20 change 4.1 对照表的 H6 ❌ 行需销账（数值注记已随复测写入）；
2. 探针复核期的**临时 `[patch]`**（`Cargo.toml` patch 到本地
   `ref/pi-shell-rs` 软链，注释即约定"复核后移除"）应移除，pi-shell-rs
   依赖更新到含 H6 修复的 rev——patch 路径依赖 `ref/`（gitignore），不移除
   而入库会破坏其它克隆；
3. 集成后 `cargo test --workspace` 对账（含 1 个负载偶发用例注记）。

## What Changes

- 移除 `Cargo.toml` 临时 `[patch]`，`cargo update -p pi-shell-rs` 更新
  `Cargo.lock` 至含 H6 修复的 rev（`eb91851`，已推 origin）。
- 4.1 对照表 H6 follow-up 销账注记（已写入 2026-11-20 change）。
- `cargo test --workspace` 全绿对账；`service_state_flip_reaches_gui_via_update_loop`
  负载偶发注记（tick 后单次断言的时序竞态，另行跟进加固）。
- 归档本 change 与已完成的 `2026-11-20-streaming-input-echo-latency`。

## Capabilities

skip_specs（集成收口与文档销账，无 spec 级行为变化）。
