# Proposal

## Why

启动链路今天是"隐式恢复"：有历史就直接把最近的工作区打开，用户没有任何选择机会，
而且没有历史时的空态选择器藏在 sidebar 里、和切换入口共用一个控件。同时切换入口
连带的三支门控（活动会话拦截 / 仅 exited 确认 / 直通）是一整套只为"中途换工作区"
存在的状态机——而单工作区模型落地后，"打开一个 workspace 后不再切换"已经是可以
明说的产品语义。

需要一个**显式的启动关卡**：启动只出 Launcher，由它完成选择/打开/创建，不允许跳过，
关掉即中断启动；主窗一旦出现就是"已经打开了的工作区"。

## What Changes

- **新增 Launcher 独立窗口**：启动阶段的唯一窗口。列出历史记录、提供打开/创建入口。
  不可跳过；关闭 Launcher 即中断启动（进程退出）；Launcher 存续期间**不启动任何其他
  逻辑**——不读历史做恢复、不 bootstrap 服务层、不装配 dock、不装菜单。
- **自动恢复退场**：`new_inner` 删除 `History::load_at` + legacy 播种 + `if let Some(path)
  = restore` 分支。历史读取与 legacy 播种整体搬到 Launcher。主窗构造改为接收 Launcher
  选定的 path。（**BREAKING**：启动不再静默直开最近工作区。）
- **删除 workspace 切换入口与三支门控**（**BREAKING**）：sidebar 顶部切换控件、
  `SidebarEvent::OpenWorkspace` / `SwitchWorkspace`、`switch_gate` / `GateDecision` /
  `count_sessions`、`request_workspace_open` 的 gated 分支全部退场。picker + marker 探测 +
  缺失确认创建那 ~70 行 rfd 胶水搬到 Launcher 实体。
- **`OpenProbe::Empty` 与 `OpenOutcome::Empty` 并入 Failure**：Launcher 已校验过 marker，
  主窗侧再失效只可能是竞态，走错误框 + 退出，与 `Failure` 合流。`OpenProbe` /
  `OpenOutcome` 各自从三臂塌成两臂。sidebar 的"无工作区"空态（`workspace-empty-hint`）
  随之成为不可达代码，删除。
- **菜单栏补 Launcher 期子句**：Launcher 期间只装 `Edit` 菜单（debug 子菜单的 action
  监听依赖 `TerminalApp`，此时不存在），主窗装配完成后整根重推带 `Debug` 的完整版——
  落在既有 "Menu content rebuild on change" 条款内。

## Capabilities

### New Capabilities

- `launcher`: 启动关卡窗口本身——历史列表呈现、打开/创建入口、不可跳过、关窗即中断启动、
  存续期间零其他逻辑。

### Modified Capabilities

- `workspace-management`: `Startup workspace restore` 三场景重写（不再直开最近工作区）；
  `Workspace switch entry` 与 `Gated workspace switching` 两条 requirement 整体删除；
  `Workspace opening via directory picker` 的入口枚举从"启动空态或切换入口"改为 Launcher。
  Purpose 段落（"list of on-disk workspaces shown in the sidebar"）为多工作区时代遗物，重写。
- `solution-service`: `Solution load failures` 中 load 边界的枚举（"startup restore,
  workspace switch, explicit reload"）随切换口删除与启动语义改变而更新。
- `project-model`: `Solution load failure blocks the workspace` 的 load 边界枚举与
  "restores the most-recent workspace at startup" 场景随启动语义改变而更新。
- `app-menu-bar`: "at startup in all build profiles" 的含义需覆盖 Launcher 期——
  明确菜单栏在 Launcher 阶段即存在（至少一个菜单），主窗装配后按既有 rebuild 条款重推。
- `dock-layout`: `Left dock is not closable` 的理由括注点名了 "workspace switch entry"，
  该控件删除后需同步措辞。

## Impact

- **代码**：`src/main.rs`（启动序列改为先开 Launcher）、`src/app.rs`
  （`new_inner` 删 restore/历史读取、`begin_open` 签名与启动 path 注入、
  `OpenProbe`/`OpenOutcome` 臂合并、`request_workspace_open` 拆分、
  `switch_gate`/`GateDecision`/`count_sessions` 删除、4 个 startup 测试迁移）、
  `src/sidebar/mod.rs`（切换入口与空态分支删除、`SidebarEvent` 7 变体降 5）、
  新增 Launcher 模块（窗口实体 + 历史列表 + picker/创建流程）。
- **测试**：`startup_restores_most_recent_workspace`、`startup_corrupt_marker_records_open_failure`、
  `startup_missing_target_falls_back_to_empty_state`、`startup_restore_load_failure_shows_error_and_no_workspace`
  四个的被测行为整体搬到 Launcher；`empty_state_renders_picker_affordance` 删除；
  `full_chain_create_add_agent_and_gate` 的链路起点断言与门控尾部删除；
  `app.rs` 中两处门控断言删除。
- **不改**：`begin_open` 的 epoch 防竞态与 `surface_fatal` 的错误框 + 退出语义、
  `commit_open` 的历史 push、`history_path` 字段、`show_add_solution` 空态
  （那是零 solution，不是零 workspace）、solution/agent 会话链路。
- **依赖**：无新增依赖。`QuitMode::LastWindowClosed` 沿用现状——Launcher 是唯一窗口时
  关窗即退出，语义天然成立；开主窗与关 Launcher 必须同批执行，否则零窗口瞬退。
