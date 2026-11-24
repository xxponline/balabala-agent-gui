# Design

## Context

`AssetManager` 是 GUI 进程内的普通 GPUI 实体：`TerminalApp` 从 `handoff()` 投影算出 owners 后直接 `asset_manager.update(cx, |m, cx| m.register(...))` 喂数据，枚举跑在 `gpui_kit::BackgroundExecutor` 上，面板通过 `manager.read(cx).state()` 拉视图。它没有契约、没有总线注册、没有 tell/镜像。

对照 `solution-service`：契约在 `crates/contracts` + crate 自己的 `contract.rs`，经 `register_contract`/`register_internal` 注册，GUI 侧在 `service_layer.rs` 持镜像，服务经 `Envelope::new(corr=None)` 的 tell 推 `TreeChanged`，写入口单点 `KIND_ACTION` + `action_id` 分派。

两条线的**动作机制也不同构**：solution 行经 `ActionInvoke` → 服务执行 → `OpResult` 回执 → 按 `action_id` 路由 notice；asset 行经 `AssetManager::execute(item_id, func_id)` 同步返回 `Vec<AssetEffect>`，由 `panel::apply_effects` 落地。`design D2` 当初的理由写在 `panel.rs:196`：*"剪贴板要窗口上下文、reveal 要系统命令，manager 保持可单测、可跨边界"*。

三个约束决定本设计：

1. **`service-layer` 是单 worker**（`runtime.rs:19` `.worker_threads(1)`），且驱动在 `loader.rs:399` 直接 `service.handle(env).await` —— 任何内联阻塞都会卡死该层全部服务。现有 `solution-service/loader.rs:51` 已在内联 `std::fs::read_to_string`（小文件，可容忍的既有隐患）；asset 枚举是全目录树遍历，**内联即事故**。
2. **`src/project_model/` 被 `launcher`/`workspace`/`solution_overview`/`service_layer`/`app` 广泛使用**，不能整体搬出二进制 crate；而 `crates/*` 库 crate 依赖不了 `src/`。
3. **剪贴板只有 GPUI 一条通路**：`cx.write_to_clipboard(&self, item)` 是 `gpui::App` 的方法，仅主线程有；项目无 `pbcopy`/`copypasta` 之类依赖。

动机见 proposal.md — Why。

## Goals / Non-Goals

**Goals:**
- `AssetManager` 成为注册进 `ServiceManager` 的总线服务，与 `solution-service` 同构（契约 / 注册 / tell 镜像 / 写入口单点）
- 枚举彻底移出 GPUI 执行器与服务 handler，单 worker 下不阻塞同层服务
- 两侧动作共用 `contracts::shared::ActionDesc`，执行归属显式声明
- `src/assets/` 退役，命名对齐（`asset-service` / `AssetOverview` / `asset-overview`）
- 能力 spec 同步改名（`asset-manager`→`asset-service`、`file-tree`→`asset-overview`）

**Non-Goals:**
- **配置面板的 flag 开关**（原⑤）—— 已拆出本 change，另立一个。现有 `SolutionConfigPanel` 是**创建表单**、仅零 solution 工作区可进，且 `CreateReq` 只有 `path` 字段，加开关属契约 BREAKING，与总线化不是同一关注点
- 不改 `solution-service` 的回执形状（`OpResult`/`OpError` 原样）
- 不动 solution 行的右键菜单行为（`execution = Service` 分支保持现状）
- 不重做 asset 枚举的过滤语义（gitignore 规则逐条照搬）

## Decisions

### D1 · 打包：`crates/asset-service/` 同时装服务与面板

仿 `crates/agent-service/` 的既定先例 —— 它的 Cargo.toml 明写 *"gpui-kit/vt100/crossterm 为 AgentTerminal 渲染层（4.1 迁入）预置"*，同一 crate 内既有 `service.rs` 的 `impl Service for AgentService`，也有 `terminal/mod.rs` 的 `impl BasePanel for AgentTerminal` 与 `cx.write_to_clipboard` 调用。

```
crates/asset-service/
  contract.rs    AssetContract + AssetRegistration DTO + TreeSnapshot/Changed + KIND_* + ActionDesc 引用
  service.rs     impl Service for AssetService — 注册 / 枚举调度 / 三态 / action_id 分派
  filter.rs      FileFilter trait + DotFilter + GitIgnoreFilter（自 src/project_model/filter.rs 迁入）
  enumerate.rs   枚举实现（自 src/project_model/enumerate.rs 迁入，改操作 DTO）
  panel/         AssetOverview（自 src/assets/panel.rs 迁入）
```

**备选与否决**：
- *只抽 `crates/asset-enumerate/`，服务留在 `src/`* —— 不成立。`register_internal` 要求 `fn(ServiceCtx) -> Box<dyn Service>`，服务必须在库 crate 里
- *面板留在 `src/assets/`* —— 可行但留下 `src/assets`/`crates/asset-service` 两处命名，且 `panel_name()`、`AssetView` 等标识符要跨两处改；D1 让 `src/assets/` 整体退役，改名单点

### D2 · 服务 / 镜像拆分，同构 `solution-service`

```
GUI ── asset.register (ask, DTO) ──────▶ AssetService
AssetService ── spawn_task ──▶ 后台枚举
AssetService ── asset.tree_changed (tell, corr=None) ──▶ GUI 镜像
AssetOverview ◀── 读镜像 ── service_layer.rs
```

tell 用 `Envelope::new(Route::Instance(watcher), …, KIND_TREE_CHANGED, Payload::new(TreeChanged(..)))`，与 `solution-service/service.rs:114` 同形；GUI 在 `service_layer.rs` 的 `listener.drain()` 循环里 downcast 收（`:420` 是 solution 的既有处理位）。

**备选与否决**：
- *GUI 继续轮询 `manager.read(cx)`* —— 保留两套机制，正是本次要消灭的
- *服务持有 `WeakEntity<TerminalApp>` 主动回调* —— 服务跨线程持 GUI 实体违反 `Service: Send + Sync + 'static`，且把 GUI 生命周期绑进服务

### D3 · DTO 是纯结构；**枚举与过滤全部归服务**（实施期修订）

`src/project_model/` 被 launcher / workspace / solution_overview /
service_layer / app 广泛使用，不能整体搬进本 crate；而枚举与过滤是纯逻辑，
应随服务走。**边界（修订后）：GUI 只出结构，服务出树。**

```text
GUI:    Project ──(纯字段映射, 不读文件)──▶ ProjectRegistration DTO
Service: DTO ──按 is_git+root 现读 .gitignore──▶ 组过滤集 ──扫盘──▶ AssetNode 树
View:   收纯数据树, 只渲染
```

```rust
pub struct ProjectRegistration { pub name: String, pub root: PathBuf,
                                 pub is_git: bool, pub modules: Vec<ModuleRegistration> }
pub struct ModuleRegistration  { pub name: String, pub root: PathBuf,
                                 pub children: Vec<ModuleRegistration> }   // ← 无 filters
```

服务侧 `ProjectEnumerator::enumerate()` 每次执行时：按 `is_git` + `root`
现读 `.gitignore` → `[DotFilter, GitIgnoreFilter?]` → 注入本次创建的模块行
→ 扫盘。

**关键收益：⟳ 天然重读规则** —— 过滤集是"每次枚举现场供给"的函数而非快照，
`refresh` 无需携带任何额外载荷，spec 的 *Refresh re-reads filter rules*
自动满足。

**而 spec 原文反而更贴**：*"**At module creation** the project … if
git-managed it SHALL read the project root's `.gitignore` and provide those
rules, **to every module it creates**"* —— 在本模型里"模块创建"字面就是
服务枚举时组装模块行那一刻；*"instead of probing the filesystem"* 仍成立
（`is_git` 由 solution 配置解析带下，服务不探 `.git`）。

**旧论证为何失效**：原 D3 说 `Module.filters` 是 `pub(super)`，转换函数
只有留在 `project_model` 子树内才读得到它——于是转换期就固化了过滤集。
DTO 去掉 `filters` 后转换只剩字段映射，该可见性约束**整条消失**，
`Module.filters` 也失去全部生产消费方 → 一并退役（实施期与用户确认）。

**备选与否决**：
- *DTO 携带 filters（原 D3）* —— 转换期固化 ⇒ refresh 用陈旧规则，
  `panel_tests::refresh_rereads_rules` 会挂；还把转换绑死在特定模块可见域
- *`KIND_REFRESH` 携带重新转换的 DTO* —— 能修好，但要给契约加字段，
  且 refresh ≈ register，语义重叠
- *DTO 只带 `root`、由服务自行解析 `is_git`* —— 违反 spec 明写的
  *"instead of probing the filesystem"*

### D4 · 枚举必须移出 handler，且 executor 彻底归调用方

单 worker + `handle().await` 直驱 ⇒ 内联枚举会停摆整层。

**运行时的真实边界**（实施期核实，`runtime.rs`）：

| 事实 | 出处 |
|---|---|
| `LayerRuntime` = `worker_threads(1)` 单 worker | `runtime.rs:19` |
| 文档明写“阻塞活走 `spawn_blocking`” | `runtime.rs:9` |
| 但 `Messaging` 只暴露 `spawn_task`，无 `spawn_blocking` | `service.rs:54` |
| `spawn_task` → `router.spawn` → **同一个单 worker** | `manager.rs:394` |

所以 `spawn_task` 只是**协作式交错**：一旦 future 内开始同步读目录，唯一 worker 就被占住。**光“移出 handler”不够，还必须把同步 fs 挪到 `spawn_blocking` 池。**

**修订（实施期）**：

1. **`Enumerator::enumerate` 改为同步签名** —— `fn enumerate(self: Box<Self>) -> Result<AssetNode, String>`
2. **offload 彻底归调用方**（这才是 D4 “executor 归调用方”的原本意图，现在才算落尽）：
   - GUI 侧 → `cx.background_executor().spawn(async move { en.enumerate() })`
   - 服务侧 → `messaging.spawn_task(async { tokio::task::spawn_blocking(move || en.enumerate()).await → tell 自己 })`
3. `asset-service` 引 `tokio`，**features 与 `agent-service` 完全一致**（其 Cargo.toml 注释已强调宿主/插件须“同一编译单元”，照抄合规）

**为什么 async 签名本来就该去掉**：旧签名 `enumerate(bg) -> BoxFuture` 里 `Box::pin(async move { bg.spawn(...).await })` —— async 只是**线程切换的载体**，`scan_root` 始终是同步 `std::fs::read_dir`（`enumerate.rs:128`）。一个“永远立即交接、自身不 await 任何真异步源”的签名是**没有兑现能力的承诺**；去掉后每个调用方反而不得不明说自己的 offload 策略。

**备选与否决（含对原否决的更正）**：

- *`tokio::task::spawn_blocking`* —— **原否决理由作废**。原文写“也可，但…且枚举体本身是 async（读目录 → 组树），非纯阻塞”，实为误判：`scan_root` 是同步 `std::fs::read_dir`，根本不是 async。**改采此项**，但需配合同步契约（否则 `spawn_blocking` 收不到 future）
- *只用 `spawn_task`* —— 协作式交错，同步 fs 仍独占 worker，达不到 spec 的 “Enumeration does not delay other services”
- *给 `service_core::Messaging` 加 `spawn_blocking`* —— 最贴合 runtime 文档意图，但 `Messaging` 是共享框架 trait，加方法要补全所有实现点（`manager.rs`×2、`catalog.rs` Noop、`solution-service/testkit`、`service-core` 测试 impl、plugin 脚手架）；**本 change 不动框架**（proposal Impact 未列），留作后续
- *枚举改真正的异步 fs* —— `std::fs` 无 async 版，得引 `tokio::fs` 或手写；而 `read_dir` 仍需包 `spawn_blocking`，多绕一层
- *维持 `BackgroundExecutor`* —— 服务用 GUI 执行器，把服务生命周期绑到窗口上，窗口关闭即枚举无人驱动

### D5 · `CopyText` 归视图：`execution = View`，不发 invoke

| 效果 | 落地代码 | 服务能否自执行 |
|---|---|---|
| `RevealPath` | `std::process::Command::new("open").arg("-R")` | ✓ 能 |
| `CopyText` | `cx.write_to_clipboard(...)` | ✗ 不能（GPUI `App` 方法，仅主线程；项目无剪贴板 crate） |

按"执行归属跟着状态走"的判据：能力在视图（剪贴板 / OS 集成）→ 视图执行；状态在服务（solution 配置）→ 服务执行。因此 asset 两个动作统一标 `View`，由 `AssetOverview` 本地执行，**不产生 invoke 消息**。

**备选与否决**：
- *effect tell（服务 tell 视图"去复制"）* —— 通道现成（`TreeChanged` 同形），但 Success 先到、复制后到，"成功"与"生效"分离，且要为一条效果单开一个 kind
- *reply 带 effect* —— 顺序最确定，但要让 asset 的回执类型与 `solution-service` 的 `OpResult` 分家（`node-actions` 对 reply 形状零约束，技术上可行），代价是两套回执
- *服务侧 `pbcopy`* —— macOS 专用 hack，绕开 GPUI 剪贴板，且非 mac 分支要另写
- *视图自行决定是否发 invoke* —— 违反 `node-actions` 原 Req 2 的 *"the view SHALL NOT encode what an action does"*，故该条已按 D6 拆分

`RevealPath` 虽服务侧也能做，但与 `CopyText` 同属 OS 集成、同批标 `View` 更一致；若将来要下沉服务侧，只需把该条的 `execution` 改成 `Service`，契约无需改动。

### D6 · `execution: ExecutedBy::Service | View` 显式、无缺省

```rust
// crates/contracts/src/shared.rs
pub enum ExecutedBy { Service, View }
pub struct ActionDesc { pub id: String, pub label: String,
                        pub checked: Option<bool>, pub execution: ExecutedBy }
```

**两侧都必须标注**：solution 动作标 `Service`，asset 动作标 `View`。

**为什么不用缺省值**：缺省若为 `Service`，asset 侧忘标会把剪贴板动作送进总线（服务执行不了，静默失败）；缺省若为 `View`，solution 侧忘标会让配置动作绕过服务直写视图（**绕过写入口单点，最坏失败模式**）。显式标注让编译期漏标即报错。

**备选与否决**：
- *视图自带已知 id 集合（`const VIEW_ACTIONS`）* —— 契约零改动，但两处真相 + 撞 id 风险：任一服务将来声明同名 id 即被视图劫持
- *按 id 前缀约定（`view.` / `service.`）* —— 同样靠约定，且 id 变更即破坏归属

连带 `node-actions` Req 2 拆为两支（service 执行 → 发 invoke；view 执行 → 不发），并新增 "View-executed actions produce no message"；`file-tree` 的 *"Activating an item SHALL dispatch back to the asset manager for execution"* 随能力退役被重写为 `asset-overview` 的新条款。

### D7 · 能力退役用 `retire_capabilities: true`

`.openspec.yaml` 支持 `retire_capabilities: z.boolean().optional()`（`dist/core/change-metadata/schema.js:51`），且 `specs-apply.d.ts:72` 明确它 gate 了退役路径。本 change 两个 REMOVED-only delta（`asset-manager` 10 条、`file-tree` 7 条）都是能力级退役，change 级设 `true`；否则归档时 REMOVED-only 会被判 sync-blocked，`openspec archive` 报 `Spec must have at least one requirement`。

新能力 `asset-service`（16 条）与 `asset-overview`（7 条）承接原 requirement，逐条在 REMOVED 的 `Migration` 里指明去处。

### D8 · 改名面与零风险点

| 层 | 现状 → 目标 |
|---|---|
| crate | — → `crates/asset-service/` |
| 服务类型 | `AssetManager` → `AssetService` |
| 面板类型 | `AssetView` → `AssetOverview` |
| 模块 | `src/assets/{manager,panel,state,traits}.rs` → 退役 |
| 字段 | `TerminalApp.asset_view`/`.asset_manager`、`shell.asset_view` → 镜像字段 |
| `panel_name()` | `"file-tree"` → `"asset-overview"` —— **全仓零调用点，零风险** |

触及 17 个 `.rs`、约 87 处引用。`AssetMenu`/`AssetEffect`/`AssetState`/`AssetNode`/`AssetKind` 随 `state.rs` 迁入 crate 或并入契约。

## Risks / Trade-offs

- **[枚举搬家破坏 `Module.filters` 可见性]** `pub(super)` 的可见域是 `project_model` 子树，`enumerate.rs` 迁出即失联 → 转换函数**必须留在 `src/project_model/` 内**（D3），由它组装带 filters 的 DTO；crate 侧只消费
- **[单 worker 阻塞复发]** 后续有人在 `handle()` 里加同步 fs → 停摆整层 → 任务里为 `AssetService::handle` 加"只做簿记、枚举必 spawn"的测试缝，并在 `service.rs` 顶部写明该约束
- **[DTO 与 `Project` 漂移]** 两处结构体并存，加字段忘改一侧 → 转换函数单点集中，DTO 字段全 pub，测试覆盖"每个 `Project` 字段都进了 DTO"
- **[BREAKING：`ActionDesc` 加字段]** `crates/contracts` 是共享契约，所有构造点要补 `execution` → `solution_actions()` 单点 + 编译期强制（无缺省），漏标即编译失败
- **[改名遗漏]** 87 处引用散在 17 个文件 → 分组改名 + 全仓 grep 收口（`AssetManager|AssetView|AssetMenu|asset_view|asset_manager|file-tree|file tree`），最后以 `cargo build` + `cargo test` 兜底
- **[能力退役的归档时序]** `retire_capabilities` 是 change 级而非 capability 级，若将来同一 change 混入"非退役的 REMOVED-only"会被误判 → 本 change 两个 REMOVED-only 均为退役，无冲突；归档时按 archive 工作流逐 capability 核对
- **[权衡：`RevealPath` 也标 `View`]** 服务侧其实能执行，统一标 `View` 换取一致性，代价是多一次视图内调用 —— 可接受，且将来改标 `Service` 不动契约

## Migration Plan

分六组，每组独立可编译、可测：

1. **契约先行** —— `contracts::shared` 加 `ExecutedBy` + `ActionDesc.execution`；`solution_actions()` 补 `Service`；全仓编译过
2. **枚举与过滤迁出** —— `filter.rs`/`enumerate.rs` 进 crate、DTO 落地、转换函数进 `src/project_model/`；此阶段服务尚未注册，行为不变
3. **服务上线** —— `service.rs` + `register_contract`/`register_internal` + `asset.register` 发送方；枚举改 `spawn_task`
4. **镜像与 tell** —— `asset.tree_changed` + `service_layer` 收包 + `AssetOverview` 改读镜像；`src/assets/manager.rs` 退役
5. **面板迁移与改名** —— `panel.rs` 进 crate 成 `AssetOverview`，`panel_name()` 改，全仓改名收口
6. **动作归属落地** —— asset 两条动作标 `View`，`apply_effects` 迁到面板，invoke 路径对 asset 关闭

**回滚**：每组独立 commit；组 3 起服务并存，若注册后异常可先摘 `register_internal` 回到组 2 状态（镜像未启用，行为等价于现状）。

## Open Questions

- **配置面板 flag 开关（原⑤）** —— 已明确拆出本 change。真正的形态（创建表单加开关 vs. 可随时编辑配置的面板）待该 change 立项时定；后者是新能力，不是"补个开关"
