# Tasks

## 1. crate 骨架与数据契约

- [x] 1.1 建 `crates/solution-service`（workspace 成员，布局对齐 agent-service：contract/service/valid_checker，WorkTree 只留注释口子不建空壳）+ `contract.rs` 数据出口（Solution item 含两配置字段、Project 描述 `{name, root, is_git}`、ask/tell `KIND_*` 常量）；验证：`cargo check -p solution-service` 通过 + DTO serde 往返单测
- [x] 1.2 solutions item 解析增两个可选配置字段（`register_to_asset_manager`/`git_enabled`）+ 缺省注入（缺失 = true）；解析语义收进 service（对齐现有 load：SolutionType registry、unknown-type、missing-path 三种错误 reason）；验证：旧格式 fixture 单测（无字段 → 双 true）、显式 false 保留、三种错误 reason 各一条

## 2. SolutionService 上总线

- [x] 2.1 `service.rs`：`SolutionService` Service impl + factory（对齐 `agent_service_factory` 形态），WorkspaceModel（solutions/projects）状态归属迁入服务端；验证：service 单测（Load/GetTree 回执、状态单份）
- [x] 2.2 `service_layer.rs` 注册 + pump 接入 + `KIND_TREE_CHANGED` 订阅镜像（拉模式纪律：无回调无跨线程合同）；验证：无头测试里 create/configure 后树变更通知到达且镜像更新
- [x] 2.3 失败回执带结构化 reason、绝不返回/缓存部分树；验证：三个 load 边界（启动 restore / 手动切换工作区 / 显式 reload）各一条测试：错误对话框出现、工作区不打开、无部分树被缓存（specs/solution-service: Solution load failures）

## 3. ValidChecker 幂等工具

- [x] 3.1 `valid_checker.rs` 纯函数幂等工具（create 同路径去重判定、configure 同值 no-op 判定）crate 根导出，View/Server 共用；验证：幂等矩阵单测（同路径/同值/先验后验）
- [x] 3.2 service 侧 Create/Configure ask 接 ValidChecker；验证：重复 Create 不产生重复 solution/project、重复 Configure 无状态变化且不重复交树（specs/solution-service: Create and configure operations are idempotent）
- [x] 3.3 View 侧发送前置检接同一 checker（`app.rs` add_solution 调用点），UI 行为不变；验证：既有 add_solution 测试全绿

## 4. git 口子与注册门控

- [x] 4.1 枚举解析产出 `Project.is_git = root 是 git 目录 AND sol.git_enabled`，随 Project 描述下发；验证：三 scenario 单测（git+enabled / git+disabled / non-git，specs/solution-service: Git-managed resolution）
- [x] 4.2 `Project::filters` 改读 `is_git` 字段，`root/.git` 运行时探测退役（`project.rs:106` 一路径）；验证：asset-manager filter 三 scenario 测试（含**行为变化**钉死：关 git ⇒ 有 `.git`/`.gitignore` 也不读，specs/asset-manager: Filter provisioning）
- [x] 4.3 `register_to_asset_manager` 门控交树：disabled 的 solution 其 project 不进交树，enable 后下次交树出现、不重启；验证：门控两 scenario 测试（specs/solution-service: Asset registration flag gates project handoff）

## 5. 交树换手与调用点迁移

- [x] 5.1 注册来源换手：Server 交 Project 描述树 → `app.rs` 转交 `AssetManager.register`，旧"app.rs 解析 projects 转 owners"路径退役；验证：asset-manager 注册生命周期测试全绿（open 全量/增量/切换替换，specs/asset-manager: Project registration lifecycle）
- [x] 5.2 add_solution 流程改走 Create ask + 交树回执（左槽 picker UI 不变）；验证：既有 `add_solution_registers_project_incrementally` 等测试全绿
- [x] 5.3 退役 `Project.next_agent_idx`/`next_agent_name` 遗留锚点（命名真值已在 AgentService，change agent-service 3.2；不引入 AllocateAgentNumber）；验证：project_model 相关单测调整后全绿 + sidebar agent 建号测试全绿
- [x] 5.4 GUI 侧 solutions/projects 读路径改走服务镜像（`Workspace.model` 字段移除），sidebar 渲染行为不变；验证：sidebar 既有测试全绿（SolutionOverview 模块重构不在本 change）

## 6. 集成核对

- [x] 6.1 全量 workspace 测试 + `cargo clippy` 触碰文件零新增告警 + `cargo build --release` 通过；验证：命令输出记录
- [x] 6.2 headed 真机冒烟并记录：workspace 打开/切换、add solution、建 agent（编号连续）、右树注册与刷新、关 git 的 solution 不读 gitignore；验证：日期 + 结果记录进本文件
      - 记录 (2026-11-23)：用户 `cargo run`（debug）真机验收通过 —
        打开/切换、add solution 项目行无重启出现、建 agent 编号连续、
        右树注册与刷新、关 git 不读 gitignore 行为抽查均无问题；
        日志 64s 无 panic/无异常 notice，term-rate 与 grid 链路正常。

## Workflow follow-up

- 归档本 change 并验证归档结果（待项目评审满足后执行）。
