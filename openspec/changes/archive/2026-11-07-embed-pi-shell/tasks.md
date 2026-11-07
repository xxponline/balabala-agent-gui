# Tasks

> 两个仓库：A = `ref/pi-shell-rs`（path 依赖，独立 git 仓）；
> B = 本仓库 `balabala-agent-gui`。组 1-2 在 A，组 3-4 在 B。

## 1. pi-shell-rs lib 化（仓库 A）

- [x] 1.1 把 `mod app` 从 `src/main.rs` 提升为 lib 公开模块（`lib.rs` 增
  `pub mod app;`，bin 改为引用 lib 路径），验证：`cargo check` 与
  `cargo test` 在仓库 A 全绿，bin 行为不变
- [x] 1.2 `draw(&mut ratatui::DefaultTerminal)` 泛型化为
  `impl ratatui::backend::Backend`（event_loop 调用点同步），验证：
  `cargo test` 中既有 `TestBackend` 快照测试全部通过
- [x] 1.3 三处真实终端 touchpoint 抽象为 sink：`TerminalGuard::enter()`
  （嵌入模式跳过 raw/alt-screen）、`SetTitle`（回调出口）、OSC52 剪贴板
  （回调出口），standalone bin 走原实现，验证：`cargo test` 全绿 +
  bin 手动冒烟确认窗口标题/复制功能不回归
- [x] 1.4 拆分 `spawn_event_sources`：暴露 key sender 供外部注入
  `UiEvent::Key`，并提供不 spawn crossterm poll 线程的嵌入入口，
  验证：新增单元测试——注入 `KeyEvent` 驱动 `handle_key` 产生预期
  intent（扩展既有 `input_tests`），`cargo test` 全绿

## 2. VirtualBackend 与 embed 入口（仓库 A）

- [x] 2.1 实现 `VirtualBackend`（实现 ratatui `Backend` trait：维护 ANSI
  输出缓冲与终端尺寸，draw 后可整帧取走），验证：单元测试——经
  `Terminal<VirtualBackend>` draw 已知内容，取回的 ANSI 序列包含预期
  文本与颜色序列
- [x] 2.2 提供 embed 会话入口：接受工作目录与 pi 二进制配置，spawn
  `pi --mode rpc` 时**设置 cwd**（现状 `rpc/process.rs` 未设置，需补
  `current_dir`），启动 app 线程、接通注入通道与 title/clipboard sink，
  并暴露只读 `SessionState` 句柄，验证：单元测试——用 `PI_BIN` 指向的
  stub 二进制断言子进程 cwd 等于传入的工作目录
- [x] 2.3 embed 冒烟测试：注入按键 → 取 VirtualBackend 帧 → 断言渲染
  文本变化（与既有 TestBackend 测试并列的回归网），验证：`cargo test`
  全绿且该测试覆盖"输入→帧"通路

## 3. GUI 侧 PiAgentShell（仓库 B）

- [x] 3.1 `AgentEntity` trait 演化：`send_input(&[u8])` 替换为
  `send_key(crossterm::KeyEvent)` + `send_text(&str)`；为保留对照，
  `PiAgentPty` 以最小 crossterm→字节编码器适配新方法，验证：
  `cargo check` 全绿（两个实现共存）
- [x] 3.2 新增 `PiAgentShell` 实现 `AgentEntity`：持有 embed handle；
  `screen()` 返回共享 vt100 屏幕（α 路线：embed 帧 → ANSI → 既有
  `vt100::Parser`）；`poll()` 吃帧并更新 parser；`send_key`/`send_text`
  转注入通道；`status()` 映射 starting/running/exited；`shutdown()`
  关闭 embed 并终止 pi 子进程，验证：`cargo check` 全绿 +
  `src/ui/pane.rs` 保持零 diff（`git diff --stat src/ui/pane.rs` 为空）
- [x] 3.3 切换 spawn 站点：`src/app.rs` 创建会话处由 `PiAgentPty::new`
  改为 `PiAgentShell::new(name, workspace_path)`，验证：`cargo check`
  与 `cargo test`（仓库 B）全绿

## 4. 集成验收与清理（仓库 B）

- [x] 4.1 按 `agent-shell` 规格场景手动验收嵌入路径：创建会话（cwd 为
  workspace 路径）、徽章三态（含 `/exit` 后 `exited`）、颜色/光标块渲染、
  键盘与 IME 提交投递（preedit 不投递）、关闭会话（活跃/非活跃），
  验证：逐条对照 `openspec/changes/embed-pi-shell/specs/agent-shell/spec.md`
  的 scenario 打勾
- [x] 4.2 全量回归：仓库 A `cargo test` + 仓库 B `cargo check/test` +
  standalone TUI bin 手动冒烟（harness 未破坏），验证：三者全绿
- [x] 4.3 对照期结束，移除 `PiAgentPty` 与 `portable-pty` 依赖
  （`Cargo.toml` diff 确认删除），验证：`cargo check` 全绿且
  `grep -r "portable_pty" src/` 无结果
- [x] 4.4 双渲染保险复查（N/A：本 change 未触碰 `view.rs`/`ViewUi`，git diff 为空）：本 change 若触碰 `pi-shell-rs` 的
  `view.rs`/`ViewUi`，确认 `view::render` 仍为
  `(SessionState, ViewUi)` 纯函数、无 `App` 隐藏读取，验证：代码
  review 记录于 PR 描述（无触碰则标注 N/A）

## Workflow follow-up

- 归档 change（`openspec archive embed-pi-shell`）并验证归档结果。
