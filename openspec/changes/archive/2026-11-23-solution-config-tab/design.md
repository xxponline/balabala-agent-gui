# Design: solution-config-tab

## Context

- 现状创建链：左槽空态 `+ add solution` → `SidebarEvent::AddSolution` →
  **rfd 系统目录 picker** → `request_create_solution`（视图侧幂等预检
  `create_is_noop` + project root 重名 → `Create` ask）→ 树镜像 →
  `sync_asset_registration`。全程无应用内 UI，预检不可见。
- 中槽机制现成（`property_detail_manager`）：`OpenObject` + `HANDLERS`
  静态注册表 + `open_tabs` 按 path 去重 + `add_panel_view(Center)` 尾插
  （console 恒 index 0）；**debug section entry point 已立先例**——非文件
  对象的签可走独立入口、独立 dedup 身份（specs/center-tabs: Debug section
  tabs…）。
- solution-service 就绪：`Create` ask/回执、`create_is_noop` 共享工具、
  树快照。测试调度器纪律：跨线程唤醒判死 → 一律 mailbox 泵拉取。
- 创建写入缺省 flags 双 true（`CreateReq` 不含 flags 字段；flags 编辑归
  将来的配置流）。

## Goals / Non-Goals

**Goals:**

- 入口动作从系统 picker 换成中槽 SolutionConfig Tab；预检（空/重名/合法性）
  可见并门控提交；结果行内呈现。
- 成功自关签、关签零写入；提交零服务/契约改动；既有
  `request_create_solution` 测试缝保持可用。

**Non-Goals:**

- 配置流（编辑既有 solution 的两 flags、`Configure` ask 的 UI）——Tab 结构
  预留 mode，落地归后续 change。
- 工作区创建 Tab、入口可见性规则（零 project 才显示）变化、
  `CreateReq` 加 flags、资产枚举/注册链改动。

## Decisions

- **D1 Tab 形态与挂载**：`SolutionConfigPanel` 实现 `PanelView`
  （`editor_panel` 同款形态），**不进 `OpenObject::HANDLERS`**——表单没有
  数据源 path 对象，`claims` 语义不匹配。走专用入口
  `TerminalApp::open_solution_config_tab`：dedup key = 固定 id
  （如 `"solution-config"`）登记进 `open_tabs`，尾插 Center；`on_removed`
  摘 dedup。再点入口命中 dedup 分支 → `select_panel` + 焦点回交（复用
  `open_object` 的既有分支逻辑）。
  备选：给表单合成一个伪 path 走 `OpenObject`——污染身份体系（path 即身份）
  ✗；继续 rfd 模态——违背 Tab 化目标 ✗。
- **D2 双栏表单与预检**：左栏 path 文本输入 + browse 按钮（rfd picker 仅
  填充输入，不再直连提交）；右栏预检行三项：**空**、**重名**
  （`create_is_noop` + 树内 project root 比对，读 `tree_view` 快照）、
  **合法性**（`exists` 且 `is_dir` 的单次 stat 探针——打字路径若不存在，
  下次装载会触发 `LoadError::MissingPath` 阻断开区，故必须门控）。
  全过才启用提交钮。**无 workspace 位置约束**（决策 2026-11-23：root 只需
  合法路径；solutions 持久化照旧在 workspace 内）。
  备选：提交时才校验——预检可见性正是本 change 的目的 ✗。
- **D3 提交链与结果**：Tab 持有 submit，复用 app 缝
  `request_create_solution(path)`（直调测试缝保留）：幂等预检 →
  `submit_solution_op(KIND_SOLUTION_CREATE)` → `kick_input_drain` 泵拉取。
  **成功支**：树镜像落地（既有 watch 镜像）→ `sync_asset_registration`
  （既有单点）→ 回执期 Tab 自关（走既有 close 路径与焦点规则）；
  **失败支**：`OpError` 消息行内显示、Tab 保留。
- **D4 入口与退役**：`AddSolution` 事件改开 Tab；`add_solution` 的
  rfd 直连提交链删除；`request_create_solution` 的 dup 早退保留
  （服务端幂等之上的双保险 + 测试缝）。
- **D5 测试策略**：全 `TestAppContext` + `run_until_parked`，无 rfd。
  场景对位：入口开签/单例聚焦/重开 fresh、预检三行与钮门控、成功自关、
  关签零写入、外部 root 接受。既有 `request_create_solution` 直调测试不动。

## Risks / Trade-offs

- [提交后立刻关签，在途 Create 回执晚到] → 回执在 app 级处理，镜像/注册与
  签无关；失败支若签已关，补落 `app.notice` 防丢。
- [每键入一次 fs stat 探针] → 单次 stat 成本可忽略；若将来输入网络盘路径
  改为失焦触发探针，不涉及 spec。
- [既有测试/注释假设「入口即 picker」] → delta 已改写对应场景；rfd 无头不
  触碰的既有约定延续。
- [RENAMED+MODIFIED 同改一条的归档语义] → 按 FROM:/TO: + 新头全块书写，
  archive 时 re-compare 校验。

## Migration Plan

单版本直接切换（入口落点变化），无数据迁移：`balabala.solutions.json`
格式与既有文件完全不动。

## Open Questions

（无：root 口径已由用户裁定 2026-11-23；创建表单不含 flags 字段为已记录
假设，flags 编辑归后续配置流。）
