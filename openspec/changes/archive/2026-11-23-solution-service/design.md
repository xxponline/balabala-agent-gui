# Design

## Context

现状（动机见 proposal.md - Why）：`app.rs` 持 `Workspace.model`（solutions + projects
+ `next_agent_idx`），workspace 打开时同步调 `project_model::load`，再把 Project
转 `Box<dyn EnumeratorOwner>` 塞给 `AssetManager`（枚举链 `Project::enumerator →
Module → 文件`，git 判定在 `project.rs:106` 运行时探 `root/.git`）。

既有分层惯例：`crates/agent-service` = `contract.rs`（线合同）+ `service.rs`
（Service impl + factory）+ 视图端代码，GUI 侧 `src/service_layer.rs` 做拉模式接入
（bootstrap 注册 / 50ms pump / `ask_core` 用户操作 / tell 只 fire-and-forget）。

已定边界（探索期决策）：
- solution-service 走 **(a) 全对齐总线服务**形态（非进程内逻辑 crate）；
- 交树边界切在 **Project 层**——solution-service 对 module 零感知，module 发现/
  过滤注入/文件枚举留在 AssetManager 之下；
- 创建流 UI 本 change 不动（左槽 picker 保留；迁入 SolutionConfig Tab 是后续 change）。

## Goals / Non-Goals

**Goals:**

- solution-service 以 agent-service 同构形态落地，Solution/Project 状态单份归属服务端
- `register_to_asset_manager` / `git_enabled` 两配置项落地并兼容旧文件（缺省 = 现行为）
- git 判定从运行时探测改为解析产出（`Project.is_git` 字段随项目下发）
- ValidChecker 幂等工具 View/Server 共用
- 三个 load 边界的错误对话框行为不回退；全量现有测试保持绿

**Non-Goals:**

- module/文件枚举迁移（明确不挪）
- WorkTree 管理（只留布局口子，不写 spec 不留空壳代码）
- UI 面变化：SolutionConfig 中槽 Tab、SolutionOverview 重构（各自独立 change）
- service status 页接入（既有 deferred 立场延续）

## Decisions

**D1: 总线服务形态（已定 a）**。`crates/solution-service/{contract,service,valid_checker}.rs`
+ factory，`service_layer.rs` 注册 + pump。备选 (b) 进程内 crate 被否：WorkTree 后续
要长驻后台任务与事件流，届时 (b) 还要再迁一次；且全生态已有 ask/tell + Listener
拉模式纪律，(a) 才是命名与形态的真正对齐。

**D2: 交树粒度 = Project 描述**（已定 b）。合同只传 `{name, root, is_git}` 级别的
项目描述树；`modules_for_enumeration`、GitIgnore 编译、文件 walk 全留
`AssetManager` 侧枚举链。备选（module 骨架 + filter 注入上移 service）被否：
module 是 solution-service 不需要知道的域，上移会逼它理解 Asset 语义。

**D3: git 判定改为解析产出**。枚举（解析出 Project）时算
`is_git = root 是 git 目录 AND sol.git_enabled`，存 `Project` 字段随树下发；
`Project::filters` 改读字段，`root/.git` 运行时探测退役。**显式行为变化**：
solution 关 git ⇒ 有 `.git` 也不读 gitignore（scenario 已钉死）。语义上
`.git` 探测属于"Project 所在根目录是否是 Git 目录"的事实判定，归 service；
`.gitignore` 读取属于 module 域，留 AssetManager。

**D4: 配置项 = solutions item 两个可选 JSON 字段**（`register_to_asset_manager`、
`git_enabled`），缺失即 true（旧行为）。备选（workspace 级默认值表）被否：
按 solution 粒度是需求本体，加一层默认值表徒增同步面。

**D5: 合同形状**。ask：Load（含三个 load 边界的同一入口）、GetTree、Create、
Configure、AllocateAgentNumber；tell：`KIND_TREE_CHANGED`（树变更通知，视图刷新）。
失败回执带结构化 reason（parse / unknown-type / missing-path），**绝不返回部分树**。
备选（Load 失败回空树 + 错误字段）被否：部分状态正是 spec 禁止缓存的东西。

**D6: ValidChecker 放 crate 根导出**。纯函数幂等工具（同路径 create 去重判定、
configure 同值 no-op 判定等），service 内部与 View 层（发送前置检）共用同一实现，
保证两端幂等语义一致。备选（View 各写一份）被否：幂等口径分叉迟早出事故。

**D7: 状态归属迁移**。`WorkspaceModel`（solutions/projects）迁入 SolutionService；
GUI `Workspace` struct 保留 path、折叠集、agents id 镜像（易失项）。
会话命名**不在**迁移范围：`pi-N` 计数真值已属 AgentService（change
agent-service 3.2，需求归属 agent-shell spec）；`Project.next_agent_idx`/
`next_agent_name` 只是 model 层遗留单测锚（生产零调用），本 change 直接
退役该字段，不引入 AllocateAgentNumber（避免命名双真值）。

**D8: 解析内核留同步入口**。`project_model` 的 parse/校验语义收进
solution-service（数据出口在 `contract.rs`），service 内部同步调用；
GUI 一律走 ask（异步回执），`app.rs` 现有打开流程改异步续接。错误对话框
渲染职责仍在 GUI（视图行为），service 只出 reason。

## Risks / Trade-offs

- [service 栈加厚一层，pdm 负载 flake 有前科] → bootstrap 只多一个轻实例；
  新增测试沿用"单测试多阶段"纪律；全量跑批若见同族 flake 先按前案归因再动
- [错误对话框三边界回归] → 每边界（启动 restore / 手动切换 / 显式 reload）各留
  一条测试，专门盯"不打开工作区 + 不缓存部分树"
- [git 行为变化混在重构里蒙过验收] → scenario 测试单列（关 git ⇒ 不读 gitignore），
  与"重构后行为不变"的回归分开跑
- [旧 solutions 文件兼容] → 解析缺省注入 + 旧格式 fixture 测试（无字段 = 双 true）
- [打开流程异步化牵动 GUI 时序] → 打开本就有加载态与失败路径；回执驱动 UI 状态，
  不引入同步等待（拉模式纪律：无回调、无跨线程合同）
- [next_agent_idx 迁移期分叉] → 计数器迁移与调用点改 ask 在同一任务内完成，
  中间态不提交

## Migration Plan

按 tasks 顺序增量落地，每步保持全量测试绿；无持久化 schema 迁移（可选字段 + 缺省）。
回滚 = revert 提交；`balabala.solutions.json` 新字段对旧二进制是未知字段，
若需回退直接删字段即可（旧行为 = 双 true 缺省）。

## Open Questions

- tell 载荷粒度（整树 vs 变更项目）留给实现期按数据量定，不影响 spec/任务划分。
