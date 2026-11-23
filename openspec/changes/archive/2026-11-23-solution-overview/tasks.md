# Tasks: solution-overview

## 1. active 投影化

- [x] 1.1 定位并列出全部 active 数据源/消费点清单（发送链 ×4、commit 清零
      ×2、poll 同步、**状态栏构造点**、测试直写/断言点），与 design D1/D5
      逐条核对后开改
- [x] 1.2 新增 `TerminalApp::active_session()` 只读访问器；发送链四处改读；
      删除 `TerminalApp.active` 字段、两处本地清零与 poll 同步块
      （design D2 行为不变论证在案）

## 2. Workspace.agents 退场

- [x] 2.1 `commit_open` 切换清场循环改从 `sessions_view()` 投影取会话 id；
      删除 `Workspace.agents` 字段与 poll 的 ids 同步块，清理
      `workspace/mod.rs` 相关导入（`collapsed_projects` 不动 — 留 UI 层）

## 3. 视图与测试缝

- [x] 3.1 sidebar `snapshot` 的 `is_active` 改为同帧读 sessions 投影
      （与 agent 行数据同一 `view`）
- [x] 3.2 测试缝归一：`inject_view` 删手工字段同步（active 走投影参数）、
      app 测试与 pdm 测试助手的 `app.active` 直写点改投影注入、断言与
      wait 条件改读 `active_session()`

## 4. 回归与验收

- [x] 4.1 workspace 全量测试零回归；`cargo build --release` 零告警；触碰
      文件 clippy 零告警
- [x] 4.2 headed 真机冒烟：键鼠输入仍到 active 会话、sidebar active 高亮、
      状态栏 `root — name [status]`、切换清场（exited 屏幕丢弃）行为不变
      （用户执行后记录）— ✅ 2026-11-23 用户 headed 验收通过（输入转发/
      grid resize 实时、"测试没有问题"）

## Workflow follow-up

- Review 通过后归档本 change（`skip_specs` — 无 delta 同步），并验证归档结果。
