# Tasks

## 1. Launcher 窗口骨架

- [x] 1.1 新增 launcher 模块（窗口实体 + 历史读取），`main.rs` 启动改为先开 Launcher；验证：启动序列无头测试断言 Launcher 是唯一窗口、主窗尚未构造、无 workspace 已打开
- [x] 1.2 Launcher 渲染历史列表（most-recent-first，`History::push` 的去重封顶语义沿用）与打开/创建入口；验证：render 级测试断言列出历史条目、空历史时列表为空、打开与创建入口均在场
- [x] 1.3 关闭 Launcher 即中断启动；验证：无头测试断言关闭 Launcher 后不存在任何窗口且没有 workspace 被打开（`QuitMode::LastWindowClosed` 语义）

## 2. 选定校验与交接（design D4）

- [x] 2.1 选定路径先跑 `marker_status`，目录不存在 / marker 缺失 / marker 解析失败时就地报错并留在 Launcher；验证：dead entry 与 corrupt marker 两个测试断言进程未退、Launcher 仍开、无 workspace 打开
- [x] 2.2 marker 缺失走确认创建（`create_workspace_files`），拒绝创建则留在 Launcher；验证：测试断言拒绝后无 workspace 且 Launcher 仍开，确认后两文件落盘
- [x] 2.3 合法路径开主窗、同批关 Launcher（先开后关，不得出现零窗口瞬退）；验证：测试断言交接后主窗存在且 Launcher 已关闭
- [x] 2.4 legacy `.workspaces.json` 播种搬入 Launcher 并在呈现历史之前完成；验证：测试断言播种后的历史条目出现在 Launcher 列表，legacy 文件未被改写

## 3. 主窗接收启动 path 与自动恢复退场（design D2/D3）

- [x] 3.1 `new_inner` 增加启动 path 参数，删除 `History::load_at`、legacy 播种与 `if let Some(path) = restore` 分支；`history_path` 字段保留供 `commit_open` 写入；验证：`cargo build` 通过，4 个 startup 测试改为显式传 path 后通过
- [x] 3.2 `OpenProbe::Empty` 并入 `Failure`、`OpenOutcome::Empty` 删除，`commit_open` 去掉 Empty 分支；验证：新增"交棒后目录失效"测试断言记录 `open_failure` 并走 fatal，而非静默回空态
- [x] 3.3 `commit_open` 的历史 push 与 `history_path` 写入不受影响；验证：`confirmed_creation_then_open_lands_workspace` 仍断言 `most_recent()` 命中该 workspace

## 4. 切换口、三支门控与空态删除

- [x] 4.1 删除 `SidebarEvent::OpenWorkspace` / `SwitchWorkspace` 与 `workspace-switch`、`workspace-empty-hint` 两个渲染块；验证：render 测试断言两块不再出现，`cargo build` 通过
- [x] 4.2 删除 `switch_gate` / `GateDecision` / `count_sessions` 与 `request_workspace_open` 的 gated 分支，picker → `marker_status` → 确认创建的 rfd 胶水归入 launcher 模块；验证：`grep -rn "switch_gate\|GateDecision\|count_sessions\|OpenWorkspace\|SwitchWorkspace" src/` 无残留，`cargo test` 全绿
- [x] 4.3 `SidebarEvent` 从 7 变体降到 5，`on_sidebar_event` 分发同步收窄；验证：编译通过且既有 sidebar 测试通过
- [x] 4.4 删除 `app.rs` 两处 `request_workspace_open(true, …)` 门控断言；验证：`cargo test` 全绿，无对已删函数的引用
- [x] 4.5 直接编辑主 spec `openspec/specs/workspace-management/spec.md` 的 Purpose 段落，去掉 "list of on-disk workspaces shown in the sidebar" 这一多工作区时代表述，改为描述 Launcher 选择 + 工作区内配置持久化；验证：Purpose 不再提及侧栏工作区列表，`openspec validate launcher-startup` 仍通过

## 5. 菜单栏两步装（design D5）

- [x] 5.1 Launcher 期直接 `cx.set_menus` 装 `Edit` 菜单（不含依赖 `TerminalApp` 的 debug action 监听）；验证：菜单测试断言 Launcher 期菜单栏至少一个菜单
- [x] 5.2 主窗 `new_inner` 尾部照旧整根重推带 `Debug` 的完整版；验证：既有 `menu_tests` 全部通过

## 6. 测试迁移

- [x] 6.1 `startup_restores_most_recent_workspace`、`startup_corrupt_marker_records_open_failure`、`startup_missing_target_falls_back_to_empty_state`、`startup_restore_load_failure_shows_error_and_no_workspace` 四个的被测行为搬到 launcher 测试（历史呈现 / 选定校验 / 关窗即退）；验证：新测试覆盖三个行为且 `cargo test` 全绿
- [x] 6.2 删除 `empty_state_renders_picker_affordance`；`full_chain_create_add_agent_and_gate` 改掉链路起点空态断言与门控尾部；验证：`cargo test` 全绿
- [x] 6.3 `open_app_with_history` 的其余调用点改为显式传启动 path；验证：`cargo test` 全绿，测试不再依赖隐式自动恢复

## 7. 集成验收

- [x] 7.1 headed 冒烟：启动出 Launcher → 选历史条目 → 主窗打开且 workspace 就位；验证：冒烟记录留存
- [x] 7.2 headed 冒烟：启动出 Launcher → 关闭 → 进程退出、无残留窗口；验证：冒烟记录留存
- [x] 7.3 headed 冒烟：启动出 Launcher → 点死条目 → 错误就地显示、留在 Launcher、进程未退；验证：冒烟记录留存
- [x] 7.4 全量回归 + `openspec validate` 通过；验证：`cargo test` 全绿、`openspec validate --change launcher-startup` 无错误

## Workflow follow-up

- Archive the change after the project's review requirements are satisfied.
- Verify the archived result.
