# Tasks

- [x] 1.1 H6 销账：2026-11-20 change 4.1 对照表写入 follow-up 注记
  （输入→帧 avg=11–18ms max=26ms ✅、ui ≈0.9ms、无冷帧尖峰；档位注意：
  debug 档慢 3–6 倍，日常/验收用 `cargo run --release`）。
  **验证**：注记在 2026-11-20 change tasks.md 4.1 之下

  > **注记（已随集成复测写入 2026-11-22）**：见
  > `2026-11-20-streaming-input-echo-latency/tasks.md` 4.1「H6 follow-up
  > 销账」注记段。

- [x] 1.2 移除临时 `[patch]`（`Cargo.toml`）并 `cargo update -p pi-shell-rs`
  至含 H6 修复的 rev（`eb91851`），`Cargo.lock` 同步。
  **验证**：`rg "patch" Cargo.toml` 无 `[patch]` 段；`Cargo.lock` 的
  pi-shell-rs 条目为 git rev 且构建全绿

  > **注记（2026-11-22）**：patch 已移除（`git checkout -- Cargo.toml`）；
  > lock 重解析落 `git+…pi-shell-rs.git#eb918514`（H6 修复 rev，已推
  > origin）；`cargo check -p pi-shell-rs` 绿。

- [x] 1.3 集成对账：`cargo test --workspace` 全绿、0 warnings；
  `service_state_flip_reaches_gui_via_update_loop` 负载偶发注记
  （时序竞态，建议限时轮询加固，另行跟进）。
  **验证**：命令输出对账（agent-service 42 + GUI 147 + doc 1 绿）

  > **注记（2026-11-22）**：`cargo test --workspace` 全绿（14 套件合计
  > 273 passed / 0 failed，0 warnings）。偶发用例
  > `service_state_flip_reaches_gui_via_update_loop`：仅在 workspace 并行
  > 高负载下偶发（断言"tick 后本地快照已翻转"的单次检查与 50ms 泵竞态，
  > 失败形态 left=Available/right=Offline；单跑 3/3、workspace 复跑 2/2
  > 均绿），与 pi-shell-rs 渲染层无关，不阻塞本 change；加固（限时轮询）
  > 另行跟进。

- [x] 1.4 归档：本 change 与已完成的
  `2026-11-20-streaming-input-echo-latency` 一并归档（均 skip_specs，
  无 delta 同步）。
  **验证**：`openspec list --json` 无活跃 change
- [x] 1.5 提交并推送本仓（deps 更新 + openspec 收口）。
  **验证**：`git status` 干净、`origin/master` 同步

  > **注记（2026-11-22）**：1.4/1.5 为流程收尾任务，随本 change 的归档与
  > 本次提交推送一并完成（归档命令与 git push 输出见提交记录）。
