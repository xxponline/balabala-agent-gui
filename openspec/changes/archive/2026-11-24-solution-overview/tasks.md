# Tasks

## 1. 树契约改形状（design D1）

- [x] 1.1 `contract.rs` 新增 `SolutionNode`/`ProjectNode`（携带 `actions: Arc<[ActionDesc]>`），`TreeSnapshot` 改为 `{ solutions, hierarchy }` 并提供 `tree()` / `handoff()` 派生方法；验证：`cargo build -p solution-service` 通过，契约单测断言派生结果与原平表等价
- [x] 1.2 `service.rs::snapshot()` 改为先算 `hierarchy` 再派生交树；验证：既有 load/configure 测试改读派生方法后全绿，注册门控收缩场景仍通过
- [x] 1.3 改 4 处非测试读点（`app.rs:727` 预检、`app.rs:772` 交树、`sidebar:137`、`solution_config:65`）；验证：`cargo build` 通过
- [x] 1.4 改 30 处测试读点与 2 处 `TreeSnapshot` 构造点；验证：`grep -rn "\.tree\b\|\.handoff\b" src/` 无残留字段访问，`cargo test` 全绿
- [x] 1.5 `last_handoff` 收窄为只比 `handoff()` 投影（字段类型改 `(PathBuf, ProjectTree)`）；验证：新增断言——仅描述符变化不触发 `sync_asset_registration`，交树变化仍触发

## 2. 服务状态与消息机制（design D3/D4/D5/D6）

- [x] 2.1 `SolutionService` 增加 `workspace_dir`，`on_load` 成功时写入、失败不更新；`CreateReq`/`ConfigureReq` 各退掉 `workspace_dir` 字段；验证：服务测试断言 load 失败后目录未被记录，create/configure 落盘仍写正确路径
- [x] 2.2 `contract.rs` 新增 `ActionDesc { id, label, checked }` 与 `ActionInvoke { node, action_id, args }`，描述符由 `snapshot()` 挂到节点上；验证：契约单测断言快照节点携带描述符与 label
- [x] 2.3 `dispatch` 改单点：删 `KIND_CREATE`/`KIND_CONFIGURE`，统一 `match action_id`；`ConfigureReq` 退 `solution_path`，目标改由 `node` 解析；验证：服务测试覆盖 create/configure 两动作，未知 action id 无状态变更且回执为失败
- [x] 2.4 新增 node 解析函数并按 D4 分流错误——格式坏走 `reject` 落死信，格式对但无此 solution 走 `OpError::NoSuch`；验证：两类各有测试，死信与红条路径分别可断言
- [x] 2.5 `request_create_solution` 改发 `ActionInvoke{action_id:"create", args:Some(CreateReq{path})}`；验证：表单提交后 solution 行出现，`create_is_noop` 前置检行为不变

## 3. 回执路由键改 action_id（design D7）

- [x] 3.1 `pending_sol_ops` 类型改 `(action_id, label)`，`take_sol_successes` 改返回 action_id；验证：编译通过，成功回执按 action_id 配对
- [x] 3.2 `app.rs:1027` / `1036` 两处按文案匹配改按 `action_id` 匹配（表单签自关、行内错误同步）；验证：新增断言——改 label 文案不影响表单签自关与行内错误呈现
- [x] 3.3 `notices` 结构化（携带 action_id）或拆双通道，`submit_solution_op` 签名改 `submit_action`；验证：`cargo test` 全绿，顶栏 notice 与表单签行内错误两个消费方都仍工作
- [x] 3.4 label 全面降为纯显示——清点 `format!("{label}失败…")` 各处确认无残留路由用途；验证：`grep -rn "contains(\"添加 solution\")" src/` 无结果

## 4. solution 行视图（design D2）

- [x] 4.1 solution 行渲染：label = 名 + 类型标记、tooltip 完整 path、id 用 `solution://` 前缀（复用 `project_id()` 同构方案）；验证：render 级测试断言行出现于其 project 之上、id 带前缀、与 project 行 id 不相等
- [x] 4.2 新增 `collapsed_solutions: HashSet<String>`（易失不持久化），solution 行左键 = 纯折叠、无打开语义；验证：测试断言折叠隐藏子行、不切 active session、不切 tab、不抢焦点；重启后展开
- [x] 4.3 右键菜单从节点 `actions` 构建（同步、零往返），点击发 `ActionInvoke`；验证：测试断言菜单项与服务声明的 label 一致、零声明时无菜单、左键不打开任何签
- [x] 4.4 空态条件从零 project 改为零 solution（project-model delta）；验证：零 solution 渲染 add-solution 入口，有 solution 后消失

## 5. 改名 sidebar → solution-overview（design D8）

- [x] 5.1 模块/类型/测试锚点改名（`src/sidebar/` → `src/solution_overview/`、`SidebarPanel`、`SidebarEvent`、`sidebar-root` 等锚点）；验证：`grep -rn "SidebarPanel\|src/sidebar" src/` 无结果，`cargo test` 全绿
- [x] 5.2 直接编辑主 spec Purpose：`openspec/specs/agent-shell/spec.md` 与 `openspec/specs/dock-layout/spec.md` 的 sidebar 措辞改 solution-overview；验证：两处 Purpose 不再以 sidebar 指称该面板
- [x] 5.3 清点 spec 正文残留：确认 11 个 requirement 块已由本 change 的 delta 覆盖、场景名因工具约束保留原文；验证：`openspec validate solution-overview` 无 error

## 6. 集成验收

- [x] 6.1 headed 冒烟：开含两个 solution 的工作区 → 两行 solution 各自带类型标记、位于其 project 之上；验证：冒烟记录留存
- [x] 6.2 headed 冒烟：右键 solution 行 → 菜单出现服务声明的开关 → 切换 → 资产树按 handoff 收缩/扩张；验证：冒烟记录留存，配置写回 `balabala.solutions.json`
- [x] 6.3 headed 冒烟：solution 行左键折叠/展开、重启后恢复展开；验证：冒烟记录留存
- [x] 6.4 全量回归 + `openspec validate solution-overview` 通过；验证：`cargo test` 全绿、validate 无 error

## Workflow follow-up

- 在 `launcher-startup` 归档**之后**归档本 change（两个 change 各改
  `dock-layout` 与 `project-model` 的不同 requirement，顺序颠倒会导致 delta
  基线漂移）。
- Verify the archived result.
