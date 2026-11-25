# Proposal

## Why

solution-service 已经持有 git 管理开关与 `is_git` 解析，但 git 能力止步于
标志位：用户无法在应用内对 git 管理的项目执行 pull/push，也无法为项目创建
worktree —— 而 worktree 正是让 agent 在独立分支上工作的容器（层级占位
`solution → project → worktree` 与 project 节点动作空位在 solution-overview
落地时就已预留）。本 change 把系统 git 封装进 solution-service 的 git
子模块，兑现 pull/push 与 worktree 骨架。

## What Changes

- **git 子模块**（`crates/solution-service/src/git/`，物理子目录）：封装
  系统 `git` 命令（`Command`，非 git2/gix），统一 stdin=null、
  `GIT_TERMINAL_PROMPT=0` 防凭证挂死，stderr 映射结构化错误。
- **pull/push**（仅 `is_git` 且 tracked-clean 的项目）：
  - 门控判据 = `git status --porcelain --untracked-files=no` 为空
    （只看 tracked 改动，不被应用自建的 worktree 目录噎死）；
  - 异步形状 = ask 秒回 `OpResult(accepted)` + `spawn_task` 后台执行 +
    **不带 corr** 的完成事件 kind → pump 按 action_id 出 notice
    （5s ask 超时内必须回执，迟到回执带 corr 会进死信）。
- **worktree 骨架**：
  - create：`git worktree add` 到 `workspace/.balabala.worktrees/<名>`，
    目录名消毒（分支斜杠/Windows 非法字符/`..`），点前缀使资产枚举默认隐藏；
  - delete：**双校验** —— ① view 侧查 `sessions_view`（project_root 匹配
    且 status ∈ {Starting, Running} 则拒，路线 A：view 编排）；
    ② git 自身裁判 —— `git worktree remove`（无 `--force`）内置
    `check_clean_worktree`（全量 porcelain，脏则原子拒删）；
  - 列表真源 = `git worktree list --porcelain`（git 元数据，不自建 manifest）。
- **层级与动作**：snapshot 增加 worktree 第三层；project 节点声明
  pull/push/create-worktree 动作，worktree 节点声明 delete-worktree 动作
  （`ActionDesc` 无 enabled 字段 → 恒声明，服务端 invoke 时门控，
  失败走既有 notice 链路）。
- **workspace-management**：兑现 "worktree data in later changes"，
  worktree 数据位置钉死在 workspace 目录内。

## Capabilities

### New Capabilities

（无 —— git 操作归 solution-service 既有边界，其 spec 已拥有
"git management" 与层级 requirement，不拆近似重复的 capability。）

### Modified Capabilities

- `solution-service`: 层级 snapshot 增加 worktree 第三层并在
  project/worktree 节点声明动作（MODIFIED 现有层级 requirement）；
  新增 pull/push 门控与异步完成回执、worktree create/delete 双校验
  两条 requirement（ADDED）。
- `workspace-management`: "Workspace data lives in the workspace directory"
  中的 "cache and worktree data in later changes" 落成实指 ——
  worktree 数据位置（`.balabala.worktrees`）钉死。

## Impact

- **代码**：`crates/solution-service/` 新增 `src/git/` 子模块与
  contract/service 扩展（新动作 id、新完成事件 kind）；
  `src/service_layer.rs`（pump 认识完成事件、delete 前置会话门控）；
  `src/solution_overview/`（worktree 行渲染）。
- **依赖边**：维持 `solution-service → service-core`（D7 纪律不破）；
  会话门控用 view 既有的 `sessions_view` 官方投影，不建 service→service 边。
- **消息契约**：新增一个不带 corr 的完成事件 kind；既有
  `KIND_ACTION`/`KIND_OP_REPLY`/`TreeChanged` 形状不变。
- **行为边界（non-goals）**：worktree 内开终端/agent 会话归
  agent-service，本 change 边界外；pull 的 merge/rebase 策略、create
  表单参数、prune 清理为后续（见 design 待定语义）。
- **平台**：Windows/macOS 双侧跑 `git.exe`/`git`（CreateProcess 之坑
  有 pi 前例可循）；`cargo test` 需真 git 造仓。
