# Tasks

## 1. git 子模块骨架（`crates/solution-service/src/git/`）

- [x] 1.1 建 `git` 子模块与统一出口 `run(args, cwd)`：`Command::new("git")`、
      stdin=null、`GIT_TERMINAL_PROMPT=0`、`GIT_ASKPASS=:`、截取 stderr 尾部；
      验证：单测断言缺 git 二进制时返回结构化错误、正常 `git --version`
      走通出口（不散落裸 Command）。
- [x] 1.2 tracked-clean 判据 `is_tracked_clean(root)`：
      `status --porcelain --untracked-files=no` 为空；验证：单测三态仓
      （干净 / 仅 untracked → 通过 / tracked 改动 → 拒）。
- [x] 1.3 `git::worktree` 封装：`add`（含目录名消毒：斜杠折叠、Windows
      非法字符替换、拒 `..` 与绝对路径）、`list --porcelain` 解析
      （path/HEAD/branch/prunable）、`remove`（无 `--force`）；
      验证：temp 仓单测 —— 建/list 见/脏树 remove 被拒且目录原封/
      干净 remove 删净，消毒用例覆盖 `feature/x` 与 `..`。

## 2. 契约扩展（contract.rs）

- [x] 2.1 动作 id 常量 `ACTION_PULL` / `ACTION_PUSH` /
      `ACTION_CREATE_WORKTREE` / `ACTION_DELETE_WORKTREE` + project 节点
      声明 pull/push/create-worktree、worktree 节点声明 delete（恒声明，
      design D5）；验证：单测断言快照中两类节点携带的 descriptor 集合
      与 label（对齐既有 `solution_actions` 测试形态）。
- [x] 2.2 层级 DTO：`WorktreeNode { path, branch, ... }`，`ProjectNode`
      增 worktree 层；验证：serde/构造单测往返不丢字段。
- [x] 2.3 完成事件 kind `KIND_GIT_OP_FINISHED`（载荷 = action_id +
      目标 + 成败 + stderr 摘要，**无 corr 字段**）；验证：单测钉
      载荷编解码与「无 corr」形态（design D1）。

## 3. pull/push 服务端（service.rs + git 子模块）

- [x] 3.1 invoke 门控：非 is_git 或 tracked-clean 不过 → 立即
      `OpError` 回执（不 spawn）；验证：单测三态 —— 干净仓接受、
      tracked 脏拒、非 git 项目拒，拒时仓库无任何改动。
- [x] 3.2 异步执行：接受后同步回 `OpResult(accepted)` +
      `spawn_task` 跑 pull/push，完成发无 corr `KIND_GIT_OP_FINISHED`；
      验证：单测（testkit mock messaging）断言回执先于完成事件、
      完成事件不带 corr、失败事件携带 stderr 摘要。
- [x] 3.3 本地 remote 集成测试：file:// 协议 temp 仓造 ahead/behind，
      pull 拉平、push 推上、无 upstream 失败走完成事件；验证：
      `cargo test` 该组全绿（无网络依赖）。

## 4. worktree create/delete 服务端

- [x] 4.1 create：`git worktree add <workspace>/.balabala.worktrees/<名>`
      （工作区目录经 load 已绑定，复用 design D5 既有绑定），完成后发
      `TreeChanged`；验证：temp 工作区集成测试 —— 建后快照见第三层、
      目录落在 `.balabala.worktrees` 下、非 git 项目拒。
- [x] 4.2 delete：服务端只跑 `git worktree remove`（无 -f），git 拒绝
      的 stderr 映射 `OpError` 回执，成功发 `TreeChanged`；验证：集成
      测试 —— 脏树被拒且目录原封、干净树删净、快照不再列出。
- [x] 4.3 列表真源：snapshot 现算走 `git worktree list --porcelain`
      （含应用外注册的 worktree，spec scenario 钉）；验证：单测 ——
      应用外 `git worktree add` 的条目也出现在其 project 下。

## 5. view 侧接线（service_layer.rs + solution_overview）

- [x] 5.1 delete 前置会话门（design D3 路线 A）：invoke 前查
      `sessions_view`（project_root 匹配 + Starting|Running → notice
      不发 invoke）；验证：单测注入两态 sessions_view —— Running 拦下
      且无消息发出、Exited 放行。
- [x] 5.2 pump 认识 `KIND_GIT_OP_FINISHED`：按 action_id 走既有
      notice 链路（成功/失败文案）；验证：单测 —— 成功与失败两态各出
      一条以 action 命名的 notice，迟到事件（原 corr 已销账）不产生
      死信噪音。
- [x] 5.3 solution-overview 渲染 worktree 行（project 下第三层 +
      右键动作菜单走既有 node-actions 通道）；验证：既有面板测试形态
      —— 有 worktree 的 project 展开见行、无 worktree 的 project 层
      为空。

## 6. 集成与回归

- [x] 6.1 全量 `cargo test` 失败集合与基线 diff 为空（本机 macOS：
      基线 0 失败，e8cb9ff 处已录）；验证：命令输出 `test result` 行
      无 failed。
- [ ] 6.2 Windows 侧冒烟（真机）：`git.exe` PATH 解析（pi 前例坑）、
      目录名非法字符消毒生效、pull/push/delete 各走一回；验证：用户
      目视 + 记录于 tasks（口径同 win-in-window 6.2）。
- [x] 6.3 macOS 侧冒烟（真机）：原生行为不回归，`.balabala.worktrees`
      不出现在资产树（点前缀过滤生效）、脏 worktree 删除被 git 拒；
      验证：用户目视 + 记录。
      **验收记录（2026-11-25, macOS 27.2.1 arm64, debug 构建, 用户真机
      目视 + 我方备环境/整理）**：
      - 冒烟环境：`/tmp/gw-mac-smoke`（git 项目 + 预建干净/脏两个
        worktree，标记/solutions 齐备，注入历史置顶打开）
      - 点前缀过滤 ✓ —— 资产树只见 `demo-proj`，`.balabala.worktrees`
        下两个完整检出未入树（默认过滤只藏 `.` 开头, spike 预期）
      - 左槽层级 ✓ —— solution → project → worktree 行（⑂ 图标 + 分支名）
      - 脏 worktree 删除被 git 拒 ✓ —— notice 带 git 原话
        "contains modified or untracked files"，目录原封
      - 干净 worktree 删除 ✓ —— 行消失 + 目录删净
      - 不回归 ✓ —— 资产树/终端/折叠正常，日志仅常态 term-rate，
        无 panic
      **后置（不阻塞本 change）**：会话前置门（task 5.1）的真机目视
      留待 agent × worktree 配合 change 一并测 —— 该门已有单测覆盖
      （Running 拦下零消息 / Exited 放行），真机需 Running 会话场景。

## Workflow follow-up

- 全部任务完成后 `openspec archive "solution-git-worktree"`，确认主
  spec 合入 solution-service 三处 delta（层级 MODIFIED + 4 ADDED）与
  workspace-management 一处 delta。
- 归档前跑 `openspec validate`。
