# Proposal

## Why

Solution/Project 域的职责今天散在 GUI 侧（`app.rs` 持 `Workspace.model`、现场调
`project_model::load`、再把 Project 转成 `Box<dyn EnumeratorOwner>` 塞给
AssetManager），Solution 的配置语义（是否注册资产、是否 git 管理）无处安放，
View 层与数据层没有可共用的幂等校验工具。要把 Solution 升为一等配置对象
（创建/配置/后续 WorkTree 管理），需要一个与 `agent-service` 形态对齐的
`solution-service` 总线服务，收回 Solution/Project 的加载、解析、配置与记账。

## What Changes

- 新增 `crates/solution-service` 总线服务（对齐 `agent-service`：contract + Service +
  factory + service_layer 接入），内部管理 Solution 与 Project（**不管 Asset/Module**）
- 新增 `ValidChecker` 独立模块：一组幂等工具，View 层与 Server 层共用
- Solution 加载与解析迁入服务：`balabala.solutions.json` → Solution → Project 枚举，
  load 失败的错误路径经 contract 回执保持逐 load 边界的错误对话框行为
- solutions item 新增配置项 `register_to_asset_manager`（是否把 Project 注册到
  AssetManager）与 `git_enabled`（是否启用 git 管理，暂只留口子）；旧文件缺省兼容
- git 判定改为解析产出：`Project.is_git = root 是 git 目录 AND solution.git_enabled`，
  随 Project 下发给 AssetManager 侧的枚举链（**行为变化**：solution 关 git ⇒
  即使 root 有 `.git` 也不读 gitignore）
- Project 注册来源换手：SolutionServer 交出已枚举好 Project 的树（仅 Project 描述层），
  GUI 转交 AssetManager；module 发现/过滤注入/文件枚举仍留在 AssetManager 之下
- **BREAKING**（内部）：`Workspace.model` 状态归属迁出 `app.rs` → 服务端；
  旧的"app.rs 解析 projects 转 owners"注册路径退役
- `next_agent_idx`（`<project>/pi-N` 编号）记账迁入服务，agent 建号走 ask

## Capabilities

### New Capabilities

- `solution-service`: Solution/Project 域的总线服务行为——加载解析与错误路径、
  状态归属与 ask/tell 合同、配置项语义（注册门控、git 口子）、幂等配置操作、
  向资产注册交树的边界

### Modified Capabilities

- `project-model`: solutions item 契约增两个配置项及其缺省语义（旧文件兼容）
- `asset-manager`: Project 注册生命周期的来源改为 SolutionServer 交树（注册门控随源生效）；
  过滤规则供给的 git 判定来源改为 Project 的解析后 `is_git` 字段

## Impact

- 新 crate `crates/solution-service`（contract.rs / service.rs / valid_checker.rs，
  预留 WorkTree 目录位）；workspace `Cargo.toml` 成员表
- `src/project_model/`：load/解析语义迁出（或收编进新 crate），`Project` 增 `is_git`
- `src/app.rs`：`Workspace.model` 迁出、注册链换手、`next_agent_idx` 调用改 ask
- `src/service_layer.rs`：注册新 service 实例 + pump 接入（tell = 树变更通知）
- `src/assets/`：注册入口改收 SolutionServer 交的 Project 描述树；`Project::filters`
  改读 `is_git` 字段（`root/.git` 运行时探测退役）
- 测试栈：service bootstrap 多一实例——pdm 负载敏感 flake 有前科，注意测试负载
