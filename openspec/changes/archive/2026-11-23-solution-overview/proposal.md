# Proposal: solution-overview

## Why

solution-service（5.4）与 agent-service 的镜像就位后，sidebar/snapshot 已
经两面读镜像，但 GUI 仍保留**两份本地投影拷贝**——`TerminalApp.active` 与
`Workspace.agents`——外加一条独立的"拷贝-比对"同步链（`poll_sessions`）。
双源意味着漂移窗口（回执与同步之间的帧）、重复同步成本，以及每次读状态时
"该信哪个"的认知负担。这是拆分表中 C（SolutionOverview 模块重构）的收口项。

## What Changes

- **`TerminalApp.active` 退场**：字段删除；读点（键鼠发送链 ×4、测试断言）
  一律改读 `service_layer.sessions_view()` 投影（新增只读访问器
  `active_session()`）；`poll_sessions` 的 active 同步删除；`commit_open` 与
  空态恢复的两处本地 `active = None` 清零删除——切换门控本就拦截活动会话
  （`switch_gate` 活动优先 Block），此处清零为冗余防御，删除后终态与过渡
  帧语义均不变。
- **`Workspace.agents` 退场**：字段删除；切换清场循环改从投影取会话 id
  （同源更新鲜）；poll 的 ids 同步块删除。
- sidebar `snapshot` 的 agent 高亮 `is_active` 改为**同帧**读 sessions 投影
  （与行数据同一 `view`，消灭帧内双源）。
- **`collapsed_projects` 留在 UI 层不动**（定案 2026-11-23：服务镜像不背 UI
  状态）。
- 测试缝归一：`test_sessions::inject_view` 的 active 参数本就是投影注入
  先例，直写 `app.active` 的测试点（含 pdm 测试助手）改为投影注入或经
  访问器断言。
- **渲染、输入转发、切换、状态栏行为全部不变**——纯数据源重构。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

（无 — `skip_specs: true`：行为不变的纯重构，无 spec 级差异）

## Impact

- **代码**：`src/app.rs`（字段、发送链读点、commit 清场、poll 同步块）、
  `src/workspace/mod.rs`（`agents` 字段）、`src/sidebar/mod.rs`
  （snapshot 的 `is_active`）、`src/service_layer/test_sessions.rs`（注入
  助手）与若干测试直写点。
- **不改**：两服务契约与服务侧真值、会话 create/close/select 链路、
  折叠态归属、`SidebarEvent` 漏斗、任何可观察渲染/交互行为。
- **测试**：断言从字段读改访问器读；注入助手改走投影；全量回归 +
  headed 冒烟（输入转发 / active 高亮 / 状态栏 / 切换清场）。
