# Design: solution-overview

## Context

两面镜像现状（勘察 2026-11-23）：

- **solution 侧**：solution-service 5.4 已收口——`Workspace.model` 移除，
  sidebar/snapshot 经 `service_layer.tree_view()` 读项目树。
- **agent 侧**：`sessions_view()` 投影已存在（`SessionsView{active,
  sessions}`，contract 真值），sidebar 行数据已按 `project_root` 分组读该
  投影；但 GUI 仍留两份**本地拷贝**：
  - `TerminalApp.active`（`app.rs:58`）：poll 同步（1042-43）、发送链读
    （309/331/349/368）、commit/空态本地清零（672/692）、测试直写
    （2518、pdm 测试助手 118、`inject_view` 176）。
  - `Workspace.agents`（`workspace/mod.rs:28`）：poll 同步（1048-49）、
    切换清场循环读（683）、`Workspace::new` 初始化。
- 测试先例：`test_sessions::inject_view(.., active_ix)` 已能直接把 active
  注入投影（line 241-246），select 回执等待条件读的也是字段（260）。
- 折叠态 `collapsed_projects` 是纯 UI 易失状态（不持久化），**留 UI 层**
  （定案 2026-11-23）。

## Goals / Non-Goals

**Goals:**

- active 与会话 id 清单**单源**（一律 `sessions_view()` 投影），删掉两份
  本地拷贝与整条 poll 拷贝-比对同步链。
- 渲染（sidebar 高亮、状态栏、console）、输入转发、切换清场行为不变。

**Non-Goals:**

- 折叠态上总线/持久化；`SessionsView` 结构或服务侧真值改造；会话
  create/close/select 链路改动；`SidebarEvent` 漏斗调整；任何 spec 行为
  变化（故 `skip_specs: true`）。

## Decisions

- **D1 只读访问器**：`TerminalApp::active_session(&self) -> Option<SessionId>`
  = `self.service_layer.sessions_view().and_then(|v| v.active)`（`active`
  为 `Option<SessionId>`，SessionId: Copy）。备选：poll 保留字段只删写点
  ——仍留双源 ✗；每次调用点内联 Option 链 ——四处热路径可读性差，且测试
  断言要长表达式 ✗。
  热路径成本：`sessions_view()` 是对常驻 `Option<&SessionsView>` 的字段
  读 + 一次 `and_then`，无锁无分配，与原字段读同阶（投影由服务线程
  mailbox → pump 单线程推进，主线程只读）。
- **D2 两处本地清零删除 = 行为不变**：
  - `commit_open(Ready)`（692）：切换三支门控**活动会话优先 Block**
    （`switch_gate(2,0)=Block`，测试在案）→ 到达 commit 时 active 必已为
    None，清零冗余；被清场的是 exited 会话（逐 id tell close），投影在关
    联回执后自然归位，与原终态一致。
  - `commit_open(Empty)`（672）：目标目录失效回空态，此刻本无会话（启动
    恢复失败路径），投影 active 同为 None。
  - spec 交叉：agent-shell 只约束 "active 存在时 console/状态栏显示什么"
    与"关闭/切换终态"，不约束 GUI 本地缓存的清零时机。
- **D3 切换清场改投影取 id**：`old.agents` → commit 时刻
  `sessions_view()` 的全部会话 id（单工作区模型下即旧工作区会话；投影为
  活数据，比 poll 拷贝**更新鲜、更全**——旧字段本就按 poll 与投影等值同步，
  改后单源）。`Workspace.agents` 字段与 poll 同步块（1048-49）随之删除。
- **D4 通知语义不丢**：原同步块靠"值变化 → `changed` →
  `notify_panels`"。删除后，active/ids 的一切真实变化都源于服务事件，
  而 `poll_sessions` 开头 `pump()` 消费事件即置 `changed` → 同一轮
  `notify_panels` 照发（含表单签，solution-config-tab 3.1 已接）；测试
  镜像注入路径本就显式 `notify`（先例）。sidebar `is_active` 与行数据同
  帧读同一 `view`，帧内自洽。
- **D5 测试缝归一**：`inject_view` 已带 `active_ix` 投影参数 —— 直写
  `app.active` 的点（app 测试 2518、pdm 测试助手 118、`inject_view` 内
  176 的"主动同步"）全部改为投影注入/无需同步（访问器直读投影，**不依赖
  pump 轮次**，比原字段更即时）；断言与 wait 条件改读 `active_session()`。

## Risks / Trade-offs

- [状态栏 active 数据源未在勘察中完全定位（构造点未命中 grep）] → task
  1.1 先行定位清单；无论数据源是投影视图还是 `app.active`，终态都归一到
  `active_session()`（spec 只约束显示内容 `root — name [status]`，不约束
  来源）。
- [切换过渡帧投影仍指旧 active（理论窗口）] → D2 论证：活动会话被门控
  拦截，commit 时投影 active 已 None；且既有切换测试守卫终态。
- [服务未起时 `sessions_view()` 为 None] → 访问器返回 None，与字段初值
  None 同义。
- [`inject_view` 不跑泵的测试依赖即时读] → 访问器直读投影，恰好消除
  "不同步就不一致"的旧行为（原本靠 176 手工同步兜底）。

## Migration Plan

单版本纯重构，无数据/文件迁移；回滚 = revert 提交。

## Open Questions

（无：折叠层归属与 skip_specs 已定案；状态栏数据源定位属 task 1.1 的
清单动作，不改变任务结构。）
