# Proposal

## Why

左槽面板今天叫 `sidebar`，但它渲染的早已不只是 session——它同时承载 workspace
入口、project 行、agent 行。而 solution 这一层在视图里**根本不可见**：
`TreeSnapshot.solutions` 只有 SolutionConfig 表单的预检在读，用户看不到自己
有哪些 solution，更没有任何入口去碰 `register_to_asset_manager` /
`git_enabled` 这两个开关——`ConfigureReq` 契约齐全、幂等工具齐全、spec 场景
齐全，**GUI 侧零调用点**。

同时，节点可用操作今天是各处硬编码：asset 侧是 `AssetManager::menu()` 里一个
`match func_id`，sidebar 侧是 `SidebarEvent` 枚举本身。操作的归属方（服务）与
操作的描述方（视图）没有分离，新增一个操作要同时改契约、服务和视图三处。

## What Changes

- **组件改名 `sidebar` → `solution-overview`**（**BREAKING** 术语）：模块、类型、
  测试锚点、spec 措辞全部跟进。spec 层面 11 个 requirement 块的正文改名 +
  `Sidebar session item display` 走 RENAMED；`agent-shell` 与 `dock-layout` 的
  Purpose 直接编辑主 spec。
- **新增 solution 行**：solution → project → worktree（后续）的层级在左槽呈现，
  solution 行 label 为 `foo · simple`（类型标记是它与下方同名 project 行的唯一
  视觉区分，因为 v1 `simple` 的 solution path 与 project root 是同一目录）。
  solution 行 = 分组头（左键纯折叠，无打开语义）+ 配置挂载点（右键菜单）。
- **树契约改为嵌套**：`TreeSnapshot.tree` 平表 → `hierarchy` 成为唯一 view
  model，`tree` / `handoff` 降为派生方法。**BREAKING** 契约形状。
- **节点操作注册机制**：服务在快照节点上携带 `ActionDesc { id, label, checked }`，
  视图据此渲染右键菜单；点击发 `ActionInvoke { node, action_id, args }` 到服务。
  label 归服务所有——契约从此携带 UI 文案。**BREAKING**：`KIND_CREATE` /
  `KIND_CONFIGURE` 退场，写入口收敛为 `solution.action` 单点。
- **服务持有 `workspace_dir`**：`LoadReq` 语义从"读文件"扩展为"绑定工作区"，
  `CreateReq` / `ConfigureReq` 各自去掉 `workspace_dir` 字段。
- **`ConfigureReq` 去掉 `solution_path`**：目标唯一来源是 `ActionInvoke.node`
  （`Option<String>`，workspace 级动作为 `None`）；服务从 node 解析目标。
- **回执路由键从 UI 文案改为 `action_id`**：消灭 `n.contains("添加 solution")`
  这类拿文案当控制流的匹配，对齐 agent 侧既有的 `CtrlOp` 枚举路由形态。
  label 降为纯显示。
- **折叠态新增 `collapsed_solutions`**：与 `collapsed_projects` 并列，同为易失
  不持久化。solution 行 id 用 `solution://<path>` 命名空间前缀，对齐既有
  `project_id()` 的 `project://` 方案。

## Capabilities

### New Capabilities

- `node-actions`: 服务注册的节点操作——描述符随快照下传、视图渲染成右键菜单、
  激活时以消息发回服务、失败以 notice 呈现。跨面板通用机制，本轮先由
  solution 侧落地。

### Modified Capabilities

- `agent-shell`: 8 个 requirement 正文的 sidebar → solution-overview 改名、
  `Sidebar session item display` RENAMED；Purpose 直接编辑主 spec；新增 solution
  行的展示、折叠、右键菜单三条 requirement。
- `solution-service`: `Solution and project state lives in the service` 扩展为
  层级快照（solution → project）并携带节点操作描述符；`Create and configure
  operations are idempotent` 的分发形态改为经 `solution.action` 单点、
  `ConfigureReq` 目标改由 node 解析。
- `project-model`: `Solution creation via the SolutionConfig tab` 的空态条件与
  成功场景随 solution 行出现而更新（零 solution 而非零 project）。
- `dock-layout`: `Left dock draggable resize` 正文改名（`Left dock is not
  closable` 由 `launcher-startup` 负责，两个 change 各改一条不重叠）；Purpose
  直接编辑主 spec。
- `theming`: `Chrome surfaces are theme-consistent` 正文改名。

**不涉及**：`workspace-management` 的 sidebar 提及由 `launcher-startup` 的
Purpose 重写一并消除；`center-tabs` / `file-tree` / `asset-manager` 均不提
sidebar。

## Impact

- **代码**：`src/sidebar/` → `src/solution_overview/`（模块、`SidebarPanel`、
  `SidebarEvent`）、`src/assets/`（`AssetMenu` 是否共用 `ActionDesc` 见 Open
  Questions）、`crates/solution-service/src/{contract,service}.rs`（层级快照、
  `ActionDesc`/`ActionInvoke`、`workspace_dir` 状态、`ConfigureReq` 退字段、
  分发单点）、`src/service_layer.rs`（`pending_sol_ops` 路由键、
  `take_sol_successes` 类型、`take_notices` 结构化）、`src/app.rs`
  （`sync_asset_registration` 收窄比较、`request_create_solution` 改发
  `ActionInvoke`、表单签自关的匹配改 action_id）、`src/solution_config/`
  （提交链路）。
- **测试**：30 处 `.tree`/`.handoff` 读点改派生方法；4 处 `TreeSnapshot` 构造
  点改 `hierarchy`；`ConfigureReq` 4 处构造点删字段；两处按文案匹配的断言改
  `action_id`；改名波及全部 `sidebar` 锚点断言。
- **依赖**：与 `launcher-startup` 无实现依赖，但 spec 层面 `dock-layout` 与
  `project-model` 各有一条 requirement 分属两个 change，须按 A → B 顺序归档
  以免 delta 叠加时基线漂移。
- **不改**：`handoff` 交树的资产注册链路语义、`begin_open`/`commit_open` 打开
  流、agent 会话的 create/select/close 链路、file-tree 右键菜单（asset 侧是否
  迁入本机制见 Open Questions）。
