# Design

## Context

- H5 期（`2026-11-20-streaming-input-echo-latency`）为验证"回显被 batch 合并
  延迟"假设，把 pi-shell-rs 临时 patch 到本地 `ref/pi-shell-rs`（软链本机
  工作区）加探针复核；约定**复核后移除**（`Cargo.toml` 注释原文）。
  该 patch 未入库（工作区改动），`ref/` 在 `.gitignore`。
- H6 根因修复落在 pi-shell-rs `2026-11-22-optimize-layout-cache-cold-frame`
  （提交 `3e4cf2e` + `eb91851`，已推 `origin/master`）；宿主复测（release
  档）达标，销账数值已注记于 2026-11-20 change 4.1 对照表。
- 本仓 pi-shell-rs 版本惯例：git 依赖 + `Cargo.lock` 锁 rev（先例
  `ae7450d` chore(deps): 更新 pi-shell-rs 538db8f -> 319dd5b）。

## 决策

1. **去 patch 而非入库**：`[patch]` 指向 gitignore 的 `ref/` 软链，入库会
   破坏其它克隆的构建；复核已完成，按约定移除，恢复 git 依赖。
2. **rev 更新方式**：`cargo update -p pi-shell-rs`（pi-shell-rs 修复已推
   origin 后），`Cargo.lock` 落新 rev，随本 change 一并提交。
3. **归档范围**：本 change 与 `2026-11-20-streaming-input-echo-latency`
   （10/10 完成、H6 follow-up 行已销账）一并归档，收口 H5/H6 全链。

## Risks / Trade-offs

- `service_state_flip_reaches_gui_via_update_loop` 在 workspace 并行高负载
  下偶发失败（断言"tick 后本地快照已翻转"的单次检查与 50ms 泵竞态；单跑/
  复跑均绿）——与 pi-shell-rs 渲染层无关，不阻塞本 change；加固（限时轮询）
  另行跟进。
- 去 patch 后本机如需再做 pi-shell-rs 本地联调，可临时恢复 patch（不入库），
  或用 `cargo update -p pi-shell-rs --precise <rev>` 对准。
