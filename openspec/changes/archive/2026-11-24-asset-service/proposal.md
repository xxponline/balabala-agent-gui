# Proposal

## Why

`AssetManager` 是 GUI 进程里的普通实体：数据由 `TerminalApp` 直接方法调用喂入，没有契约、没有总线注册、没有 tell/镜像，右键菜单走 `AssetMenu`/`AssetEffect` 私有类型而非 `contracts::shared::ActionDesc`。结果是同一个 GUI 进程里两个面板的数据属主走两套机制——solution 行经 `ActionInvoke` + `action_id` 路由到服务执行，asset 行经 `func_id` → `Vec<AssetEffect>` 本地分派。同时命名已经名实分离到该清理的程度：`file-tree` 这个 spec 名从无对应模块，`asset-manager` 这个 id 就是旧类型名派生的。

## What Changes

- **`AssetManager` 上总线**：新建 `crates/asset-service/`，仿 `crates/agent-service/` 的既定模式——同一 crate 内装总线服务（`impl Service for AssetService`）与 GUI 面板（`impl BasePanel for AssetOverview`），按需依赖 `gpui-kit`。经 `register_contract("asset", …)` + `register_internal("asset", factory, …)` 注册进 `ServiceManager`。
- **拆成服务 + GUI 镜像**，同构 `solution-service`：GUI 侧经 `asset.register` 发 DTO（`Project`/`Module` 不整体搬出 `src/project_model/`），服务侧枚举后经 `asset.tree_changed`（tell）推镜像，`AssetOverview` 读镜像渲染。
- **枚举移出 GPUI 执行器**：`service-layer` 运行时是 `worker_threads(1)` 单 worker 且 `service.handle(env).await` 直接跑在其上，全目录遍历必须经 `messaging.spawn_task` 移出 handler，否则卡死全部服务。`gpui_kit::BackgroundExecutor` 签名随之替换。
- **动作执行归属显式化**（**BREAKING** 契约）：`contracts::shared::ActionDesc` 新增 `execution: ExecutedBy::Service | View`，**两侧都必须显式标注、无缺省值**——避免"忘标"静默降级成视图执行。
  - solution 动作标 `Service` → 发 invoke → 服务执行（现状不变）
  - asset 动作（复制路径 / 在 Finder 显示）标 `View` → **视图直接执行、不发消息**，因为剪贴板与 OS 集成本就是 view 层能力；服务的声明只提供菜单条目与标签
- **`src/assets/` 整体退役**，面板迁入新 crate 并改名 `AssetOverview`；`panel_name()` 由 `"file-tree"` 改为 `"asset-overview"`（现为零调用点）。
- **能力重命名（退役 + 新建）**：`asset-manager` → `asset-service`、`file-tree` → `asset-overview`。

**明确不在本 change**：配置面板的 flag 开关（原⑤）已拆出另立 change —— 现有 `SolutionConfigPanel` 是创建表单、仅零 solution 工作区可进，且 `CreateReq` 只有 `path` 字段，加开关属契约 BREAKING，与总线化不是同一关注点。详见 design.md Non-Goals。

## Capabilities

### New Capabilities

- `asset-service`: 右槽资产数据的总线服务——注册、异步枚举、项目根相对过滤、三态视图、tell 推镜像、以及节点动作的声明与执行归属。
- `asset-overview`: 右槽面板——从快照构建右键菜单、左键打开钩子、视图层动作的本地执行、刷新时机、空态与降级表现。

### Modified Capabilities

- `asset-manager`: **退役**（全部 requirement REMOVED），职责由 `asset-service` 承接。
- `file-tree`: **退役**（全部 requirement REMOVED），职责由 `asset-overview` 承接。
- `node-actions`: `ActionDesc` 增 `execution` 字段；"Activating an action messages the declaring service" 拆为服务执行 / 视图执行两支，视图执行的动作不产生消息。
- `dock-layout`: 右槽渲染内容的措辞由 "file tree panel" 改为 "asset-overview"（3 处）。
- `project-model`: `Solution configuration file` 中 "asset manager" 措辞更新。（原拟新增的配置面板 flag 开关需求已拆出，不属本 change。）
- `solution-service`: `Asset registration flag gates project handoff` 中 "asset manager" 措辞更新。
- `theming`: `Chrome surfaces are theme-consistent` 中 "file tree" 措辞更新。

## Impact

- **新增** `crates/asset-service/`（contract / service / panel / enumerate），依赖 `service-core` + `gpui-kit`
- **迁入** `src/project_model/{enumerate,filter}.rs` 的枚举与过滤逻辑（经 DTO，`Project`/`Module` 留在 `src/project_model/`）
- **退役** `src/assets/`（`manager.rs` / `panel.rs` / `state.rs` / `traits.rs` 及两份测试）
- **改动** `src/service_layer.rs`（注册 + 镜像 + 效果落地）、`src/app.rs`、`src/shell/mod.rs`、`src/solution_config/`
- **契约** `crates/contracts/src/shared.rs` 增 `ExecutedBy`；`crates/solution-service` 全部 `ActionDesc` 构造点补 `execution`
- **改名触及** 17 个 `.rs`、约 87 处引用；spec 措辞 6 个 capability、约 22 处
- **预估 40+ tasks**（对比 `solution-overview` 25）
