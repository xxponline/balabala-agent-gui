# Tasks

## 1. 契约与动作归属（BREAKING 前置）

- [x] 1.1 在 `crates/contracts/src/shared.rs` 增 `pub enum ExecutedBy { Service, View }` 并给 `ActionDesc` 加 `pub execution: ExecutedBy` 字段 — 验证: `cargo build -p contracts` 通过
- [x] 1.2 给 `crates/solution-service` 内全部 `ActionDesc` 构造点补 `execution: ExecutedBy::Service`（`solution_actions()` 及 `contract.rs`/`service.rs` 测试） — 验证: `cargo test -p solution-service` 全绿
- [x] 1.3 补契约测试断言 solution 节点声明的两个动作 `execution` 均为 `Service`，并确认漏标字段会编译失败（无缺省值） — 验证: `cargo test -p solution-service` 中新测试通过
- [x] 1.4 在 `ActionDesc`/`ExecutedBy` 的 doc 注释写明两值语义与"为什么必须显式"（忘标 = 最坏失败模式） — 验证: `cargo doc -p contracts --no-deps` 无告警

## 2. 枚举与过滤迁出 + DTO

- [x] 2.1 新建 `crates/asset-service` 骨架（`Cargo.toml` 依赖 `service-core` + `gpui-kit`，`lib.rs` 声明 `contract`/`service`/`filter`/`enumerate`/`panel` 模块） — 验证: `cargo build -p asset-service` 通过
- [x] 2.2 迁 `src/project_model/filter.rs` → `crates/asset-service/src/filter.rs`（`FileFilter` trait + `DotFilter` + `GitIgnoreFilter`），`project.rs` 改为 `use asset_service::filter::*` — 验证: 原 filter 测试随迁后 `cargo test -p asset-service` 通过
- [x] 2.3 在 crate 内定义 `ProjectRegistration` / `ModuleRegistration` DTO（字段全 `pub`、`modules` 递归；**纯结构 — 不含 `filters`**，实施期修订 design D3：过滤集由服务在每次枚举时按 `is_git`+`root` 现场供给，否则 ⟳ 会用陈旧规则） — 验证: `cargo build -p asset-service` 通过
- [x] 2.4 迁资产树 DTO 与枚举契约入 crate 并按 design D4 去掉 GPUI 执行器：`AssetKind`/`AssetNode`/`project_id` → `contract.rs`；`BoxFuture`/`EnumeratorOwner`/`Enumerator` → `enumerate.rs`，签名 `enumerate(bg: BackgroundExecutor)` 改为 `enumerate()`；`src/assets/state.rs` 拆分（`AssetState`/`AssetMenu`/`AssetEffect`/`find_kind` 留在 `src/assets/` 待 4~6 组）、`src/assets/traits.rs` 退役；同步改 `AssetManager::spawn_enumerate` 与 `enumerate.rs` 调用点 — 验证: `cargo build` 零告警且 `cargo test -p asset-service` 通过
- [x] 2.5 迁 `src/project_model/enumerate.rs` → `crates/asset-service/src/enumerate.rs`，把 `ProjectEnumerator { project: Project }` 改为操作 `ProjectRegistration`（`Project`/`Module` 的 `EnumeratorOwner` 实现随迁为 DTO 版） — 验证: `enumerate.rs` **自己的 14 个测试**（`file_enumerator_*`、`module_row_*`、`pruning_*`、`rules_evaluate_*` 等）在 crate 内通过
- [x] 2.6 在 `src/project_model/` 内写转换函数 `Project → ProjectRegistration`（**纯字段映射、不读文件**；原“必须留在本子树才能读 `Module.filters` 的 `pub(super)`”的约束随 filters 移出 DTO 而整条消失 — 实施期修订，`Module.filters` 随之退役） — 验证: 新增测试"每个 `Project`/`Module` 字段都进了 DTO"通过
- [x] 2.7 从 `src/project_model/mod.rs` 摘除 `pub mod enumerate;`（`pub mod filter;` 已随 2.2 摘除），更新模块注释写明新依赖方向（`src/` → `crates/asset-service`） — 验证: `cargo build` 通过且无 `unused` 告警
- [x] 2.8 确认 `src/project_model/files.rs` 不动（工作区 marker/solutions 契约，与枚举无关） — 验证: `grep -q "files" src/project_model/mod.rs` 仍在，`cargo build` 通过

## 3. 服务上线

- [x] 3.1 写 `crates/asset-service/src/contract.rs`：`AssetContract`、`AssetRegistration`、`TreeSnapshot`/`TreeChanged`、`KIND_REGISTER`/`KIND_TREE_CHANGED`/`KIND_ACTION` — 验证: `cargo build -p asset-service` 通过
- [x] 3.2 写 `crates/asset-service/src/service.rs` 的 `impl Service for AssetService`：`start()` 载入、`handle()` 按 kind 分派、注册集替换、三态（Empty/Degraded/Forest）、视图代数 — 验证: `cargo test -p asset-service` 中服务级测试通过
- [x] 3.3 **`handle()` 只做同步簿记**，枚举体移出 handler（design D4 修订版）：先把 `Enumerator::enumerate` 改为**同步签名** `-> Result<AssetNode, String>`、GUI 侧改 `bg.spawn(async { en.enumerate() })`；服务侧 `spawn_task(async { tokio::task::spawn_blocking(move || en.enumerate()).await → tell 自己 })`，并给 `asset-service` 加与 `agent-service` features 完全一致的 `tokio` 依赖 — 验证: 新增测试"枚举在途时发往另一服务的消息不被阻塞"通过，且 `cargo build` 零告警
- [x] 3.4 在 `src/service_layer.rs::bootstrap` 注册 `register_contract("asset", …)` + `register_internal("asset", factory, …)` — 验证: `cargo run` 启动后 `ServiceManager` 状态含 `asset` 且为 `running`
- [x] 3.5 补服务生命周期测试：工厂产出的实例 `start()` 返回 Ok、未知 kind 走 reject/dead-letter、未注册 workspace 就收到 register 的结构化错误 — 验证: `cargo test -p asset-service` 通过

## 4. 镜像与 tell

- [x] 4.1 服务在视图落地时发 `asset.tree_changed` tell（`Envelope::new(corr=None)`，形如 `solution-service/service.rs:114`） — 验证: `cargo test -p asset-service` 中"落地即推一条、输入未变不推"测试通过
- [x] 4.2 在 `src/service_layer.rs` 的 `listener.drain()` 循环收 `KIND_TREE_CHANGED` 写入 `asset_view` 镜像字段 — 验证: 新增测试"枚举落地后镜像可读且内容完整"通过
- [x] 4.3 GUI 侧把 `handoff()` 投影转成 `ProjectRegistration` DTO 并发 `asset.register`（取代 `asset_manager.update(cx, …register…)` 直调），⟳ 改发 `asset.refresh`；**并补 `asset.watch` 握手**（`pump()` 见 asset Available 时握手，同 agent/solution 时序 —— 缺它则 `emit` 无订阅者、镜像永远 `None`，属 design D2「tell 推镜像」的必然后果，原任务未列） — 验证: `cargo test` 中 `add_solution_registers_project_incrementally` 仍通过
- [x] 4.4 让 `AssetOverview` 改读镜像而非 `manager.read(cx)`（`state()`/`view_generation()` 改由 `service_layer.asset_view()` 供给） — 验证: `cargo test` 中面板渲染测试通过
- [x] 4.5 退役 `src/assets/manager.rs`（含 `manager_tests.rs` 11 个测试随迁或改写；**`panel_tests` 亦需随之迁移** —— 它的 `manager()`/`register_ws` 观测缝与同步时序前提都变了） — 验证: `grep -r "AssetManager" src/` **代码引用**为零（注释中作为「继任者/已退役」的迁移说明保留），`cargo build` 零告警、`cargo test --workspace` 通过
  - ⚠ **4.3/4.4/4.5 是一次原子迁移**（实施期发现）：`AssetManager` 同时是数据汇与面板源，只改输入则面板读不到、只改面板则镜像没人填 ⇒ 三者必须一起落地、全绿后一并勾选
- [x] 4.6 补镜像一致性测试：切换工作区后镜像被整体替换、无残留旧行 — 验证: `cargo test` 中新测试通过

## 5. 面板迁移与改名

- [x] 5.1 迁 `src/assets/panel.rs` → `crates/asset-service/src/panel/`，类型 `AssetView` → `AssetOverview` — 验证: `cargo build -p asset-service` 通过
- [x] 5.2 `panel_name()` 由 `"file-tree"` 改为 `"asset-overview"` — 验证: `grep -rn '"file-tree"' src/ crates/` 为零
- [x] 5.3 改字段与引用：`TerminalApp.asset_view`/`asset_manager`、`shell.asset_view`、`app.rs` 各调用点 — 验证: `grep -rn "asset_view\|asset_manager" src/` 为零
- [x] 5.4 全仓改名收口：`AssetManager` / `AssetView` / `src/assets` 代码引用清零、`src/assets/` 目录删除（`AssetMenu`/`AssetEffect` **已随面板迁入 crate** —— 它们要到 group 6 才换源：菜单改用 `ActionDesc`、效果落地见 6.2，故其引用**不应**在 5.4 归零） — 验证: `grep -rn "AssetManager\|AssetView\|src/assets" src/ crates/` **代码引用**为零，`cargo build` 零告警
- [x] 5.5 `panel_tests.rs`（21 个测试）随面板迁移 —— **修订（方案 A）：留在 `src/shell/asset_overview_tests.rs` 作为 bin 侧集成测试**，而非迁入 crate。理由：它们驱动 `TerminalApp::new`/`begin_open`/`request_create_solution` 等二进制类型，crate 依赖不了；crate 不该为测试反向依赖宿主。连带：面板观测字段（`expanded`/`dir_ids`/`last_*`/`view_pulls`）改为 `pub`（bin 依赖的 crate 不带 `cfg(test)`，测试缝必须常驻） — 验证: `cargo test --workspace` 通过
- [x] 5.6 更新 `src/shell/mod.rs`、`src/app.rs` 中"右槽数据服务层/文件树面板"等注释措辞 — 验证: `grep -rn "文件树面板\|file tree" src/` 为零

## 6. 动作归属落地

- [x] 6.1 给 asset 两个动作（复制路径 / 在 Finder 显示）的 `ActionDesc` 标 `execution: ExecutedBy::View`，标签仍由服务声明 — 验证: `cargo test -p asset-service` 中"节点携带两个 View 动作"测试通过
- [x] 6.2 把 `apply_effects`（`CopyText` → `write_to_clipboard`、`RevealPath` → `open -R`）迁到 `AssetOverview` 内执行，asset 动作**不发 invoke** — 验证: 新增测试"右键激活复制路径后剪贴板内容正确且发往服务的消息数为 0"通过
- [x] 6.3 项目行不声明动作 → 右键无菜单 — 验证: `cargo test` 中新测试通过
- [x] 6.4 更新 `crates/asset-service` 模块级注释写明"执行归属跟着状态走"的判据（design D5） — 验证: `cargo doc -p asset-service --no-deps` 无告警

## 7. 集成验证

- [x] 7.1 全量 `cargo test` 通过（含已知 flaky：`status_badge_refreshes_via_drain_loop`、`create 会话超时` 单跑复核） — 验证: `cargo test` 输出无新增失败
- [x] 7.2 `cargo build` 无新增 `warning`（`unused`/`dead_code` 尤其） — 验证: `cargo build 2>&1 | grep -c warning` 不高于改动前基线
- [x] 7.3 `openspec validate asset-service --strict` 通过 — 验证: exit 0
- [x] 7.4 全仓 grep 收口复核：`AssetManager|AssetView|AssetMenu|asset_view|asset_manager|file-tree|file tree|src/assets` 全为零 — 验证: 上述 grep 输出为空
- [x] 7.5 复核 spec 措辞已落地：`openspec/specs/` 暂不改动（同步在归档时做），但 delta 内 `node-actions` 两支拆分与 `file-tree` 退役的 `Migration` 指向逐一对应 — 验证: 对照 `openspec/changes/asset-service/specs/*/spec.md` 逐条人工核对

## 8. Headed smoke（需人工执行 — 助手无屏幕录制/辅助功能权限）

- [x] 8.1 打开工作区，确认右槽 asset 树正常渲染、项目/模块/文件层级与刷新按钮工作，⟳ 后新文件出现 — 验证: 人工观察 + 截图
- [x] 8.2 右键文件行 → 菜单出现「复制路径」「在 Finder 显示」两项；点复制后粘贴得到绝对路径；点 reveal 后 Finder 定位到该文件 — 验证: 人工操作确认
- [x] 8.3 右键 project 行 → **无菜单**；solution 行右键 → 两个开关（注册到资产树 / git 管理）照旧可用且勾选态正确 — 验证: 人工操作确认
- [x] 8.4 枚举大目录时（如指向含大量文件的项目）终端输入与 solution 行点击仍流畅，无卡顿 — 验证: 人工观察
- [x] 8.5 关闭右槽、切主题、切工作区各一轮，asset 树与空态/降级态表现正常 — 验证: 人工观察

## Workflow follow-up

- 归档前先跑 `openspec instructions archive --change "asset-service" --json` 取归档输入，再按 archive 工作流逐项核对（artifact 完成 → task 完成 → delta 同步评估）
- 同步阶段注意：`asset-manager` 与 `file-tree` 是 REMOVED-only + `retire_capabilities: true` 的**能力退役**，按 archive 规则应判为已同步、不得重建主 spec；`asset-service`/`asset-overview` 为新能力，从 ADDED 创建
- 归档目标名 `2026-11-24-asset-service`（若跨日则按当天日期）
- 归档后删除临时备份（若有）并复核 `openspec list --specs` 含 `asset-service`/`asset-overview`、不含 `asset-manager`/`file-tree`
