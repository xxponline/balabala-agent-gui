# Design

## Context

- solution-service 是总线服务：driver 单线程顺序 dispatch、同步处理 +
  同步回执；`handle()` 返回 `ServiceFuture`，慢操作会把 load/get_tree
  全堵住（顺序背压）。ask 超时 `DEFAULT_CALL_TIMEOUT = 5s`
  （service-manager），**迟到回执按 corr 销账失败进死信**
  （flow_tests.rs:269 "late or unknown reply"）。
- 既有钩子：project 节点 `actions = []`（service.rs:73 注释 "create
  worktree 属后续"）；层级占位 `solution → project → worktree` 与
  `args` 通道（solution-overview design）；workspace-management spec
  预言 "cache and worktree data in later changes"；single-workspace
  design:259 悬置 `.balabala` 前缀命名待 worktree change 定名。
- `is_git = root/.git exists AND solution.git_enabled`（loader.rs:71）
  是现成门控。`ActionDesc` 只有 `id/label/checked/execution`，无
  enabled 字段（contract.rs:51-60）。
- view 侧已有 `sessions_view` 官方投影（service_layer.rs:471，
  agent-service `sessions_changed` 推送），本职渲染源；
  `SessionInfo { project_root, status: Starting|Running|Exited }`。
- 全仓无 service→service 先例；依赖纪律 D7：solution-service 只依赖
  service-core。参考 `ref/git`（git 源码）可直接判例。

## Goals / Non-Goals

**Goals:**

- git 子模块（`crates/solution-service/src/git/`）封装系统 git：
  pull/push + worktree create/list/delete。
- 层级 snapshot 长出 worktree 第三层；project/worktree 节点声明动作。
- 双门：pull/push 门 = is_git AND tracked-clean；delete 门 = view 会话
  门 + git 自身 clean 检查。

**Non-Goals:**

- worktree 内开终端、agent 会话归属 worktree —— agent-service 地盘
  （后续 change，届时会话门从 view 移入服务端的动机才真实存在）。
- pull 的 merge/rebase 策略选择、create 表单的完整参数集、prune
  清理入口、删除顺手 `branch -d` —— 见 Open Questions。
- `ActionDesc` 增加 enabled 字段（方案 b 已规避，UI 灰显留后续）。

## Decisions

### D1 路线：pull/push 异步形状 = 秒回 accepted + spawn_task + 无 corr 完成事件

```
view --ask(KIND_ACTION, corr)--> service
      |-- 立即回 OpResult(accepted)  (corr 在 5s 内销账, 无死信)
      +-- spawn_task 后台跑 git pull/push (不占 driver 顺序上下文)
             |-- 成功: tell 新 kind (无 corr) --> pump --> notice
             +-- 失败: 同 kind + action_id + stderr --> notice
```

- **为什么不能同步等**：① 5s ask 超时 → UI 只见 "ask timeout"；
  ② 迟到回执带原 corr → 死信；③ driver 顺序 await → load/get_tree
  被网络操作堵死。
- **为什么完成事件不带 corr**：原 corr 已销账，router 对无 pending 的
  回复判 "late or unknown reply" 死信。无 corr 的 tell 不进销账表。
- **备选**：(b) 加长 ask 超时 —— 被否，router 超时是全局构造参数，
  单操作豁免要动契约且 UI 仍被挂 30s；(c) 轮询状态 —— 被否，违背
  既有 tell/Listener 拉模式纪律的推送习惯。
- worktree create/delete 改层级 → 完成后附带发既有 `TreeChanged`
  （复用，不新造）；pull/push 不改层级 → 只发新完成事件 kind。

### D2 门控判据：pull/push 用 tracked-clean，delete 让 git 裁决（判据刻意不同）

| 门 | 判据 | 裁决者 |
|---|---|---|
| pull/push | `git status --porcelain --untracked-files=no` 为空 | 我方（invoke 前查） |
| delete | 全量 porcelain（含 untracked/子模块） | **git 自己**：`worktree remove` 无 `--force` |

- **pull/push 为什么 tracked-only**：spike 实证 workspace 根 == project
  根时，应用自建的 `.balabala.worktrees/` 出现在主仓 porcelain
  （`?? .balabala.worktrees/`），全量判据会让门**永不通过**（功能
  自噬）。tracked-only 同时吻合「已提交干净」语义（untracked 是未纳管
  新文件，不是未提交改动）。
- **delete 为什么不自研**：`ref/git/builtin/worktree.c:1372`
  `check_clean_worktree` 就是判例 —— `status --porcelain
  --ignore-submodules=none` 读到任何输出即 die，**先查后删原子**
  （脏则一个字节不动）。自研 = 判据漂移风险；直接调 `worktree remove`
  （无 -f）= 门与执行器合一，白捡 locked worktree / 主 worktree 两个
  拒删情形。
- **备选**：pull/push 也全量 porcelain + 自动写 `.git/info/exclude`
  —— 被否，动用户仓库的 git 配置越权；tracked-only 无需副作用。

### D3 删除双校验的归属：路线 A（view 编排）

```
delete invoke 发出前:
  service_layer 查 sessions_view
    有 project_root 匹配 && status ∈ {Starting, Running} -> 直接 notice, 不发 invoke
  通过 -> KIND_ACTION(delete) -> 服务端只跑 git worktree remove
```

- **为什么 view**：① 生态既有形态（打开链路/create/configure 全是
  view 居中编排）；② `sessions_view` 是 agent-service 亲自推送的官方
  投影，非 view 私自抄状态；③ 不破 D7（零 service→service 边）；
  ④ worktree 会话本体在边界外，TOCTOU 窗口的实际后果目前仅为
  「删 worktree 时项目根有个 agent 在跑」，风险敞口小。
- **备选**：(B) solution-service 订阅 sessions_changed 自持镜像 ——
  无 TOCTOU 但开全仓第一个 service→service 边，等 worktree 会话进场
  再议；(C) invoke 内同步 ask agent-service —— 被否，5s 超时 + driver
  背压被跨服务拖累 + 双向类型依赖。
- **已接受的残余**：A 的 TOCTOU（校验与 invoke 之间 session 可启动）。

### D4 目录：`workspace/.balabala.worktrees/<消毒名>`，点前缀 + tracked-only 双解药

spike 实证（tmp 仓，已清理）：

```
实验                                    结果
worktree add 建在主仓 working tree 内    git 不拒绝
主仓 status --porcelain                ?? .balabala.worktrees/   (点前缀救不了 git)
主仓 status --untracked-files=no        空                        (tracked-only 救得了)
资产枚举默认过滤只藏 . 开头              点前缀救得了资产树
worktree remove 无 -f 脏树              fatal + 目录原封 (原子)
remove 干净树                          目录+元数据删净, 分支残留
手删目录后 list --porcelain             出现 "prunable: gitdir file ..." 标记
```

- 两个消费者两种污染，**缺一不可**：点前缀 → 资产枚举默认隐藏；
  tracked-only → pull/push 门免疫（D2）。
- 点前缀同时兑现 single-workspace design:259 悬置的 `.balabala`
  命名决定。
- 目录名消毒：分支名斜杠折叠、Windows 非法字符（`:*?"<>|`）替换、
  拒 `..`/绝对路径；同名 project 消歧（`<project>-<ref>` 撞了加短
  hash 后缀 —— 具体规则 tasks 落地时钉）。
- **元数据真源 = git**：`<project>/.git/worktrees/<id>`，
  列表用 `git worktree list --porcelain`（可发现应用外注册的
  worktree，spec scenario 已钉），不自建 manifest。

### D5 动作声明：恒声明 + 服务端 invoke 门控（方案 b）

- `ActionDesc` 无 enabled 字段 → 备选 (a) 快照期按状态声明会被
  invoke 前的状态变化打脸（陈旧）；(c) 给 ActionDesc 加字段动公共
  契约、波及全部服务。
- 方案 b：project 节点恒带 pull/push/create-worktree、worktree 节点
  恒带 delete，门控在服务端（pull/push）或 view（delete 会话门，
  D3），不过 → `OpError` 回执 → 既有 notice 链路（node-actions
  "failures surface as a named notice"，按 action_id 路由）。

### D6 git 子模块形态：系统命令封装，防挂三件套

- `Command::new("git")`（非 git2/gix）：零新依赖、行为与用户终端
  一致、`ref/git` 可直接当判例源。
- 防挂：`stdin = null`（绝不读输入）、`GIT_TERMINAL_PROMPT=0` +
  `GIT_ASKPASS=:`（凭证请求即失败而非挂死后台任务）、stderr 截断
  保留尾部（git 的错误在尾部）。
- Windows：`git.exe` PATH 解析有 pi 前例（agent-service/shell.rs:98
  CreateProcess 坑）同场验证。
- 测试：cargo test 内真 git 造仓（`git init` + 本地 remote 走
  file:// 协议，无网络），temp 目录形态沿用既有 testkit 风格。

## Risks / Trade-offs

- [5s 超时内 accepted 回执若被 spawn_task 排队拖慢] → spawn_task 交
  宿主 runtime 即刻返回，accepted 在 handle() 同步路径发出，不排队。
- [后台 git 任务 panic 拖垮宿主] → spawn_task 语义按 D12 约定由宿主
  按 crash 上报（既有约定，不新造）。
- [凭证/交互挂死] → D6 防挂三件套；任何 git 子进程必经统一
  `git::run()` 出口，不散落裸 Command。
- [TOCTOU：会话门校验后 session 启动] → 已接受（D3），worktree 会话
  进场时把门移入服务端。
- [有子模块的项目 delete 恒被 git 判脏] → git 保守立场
  （validate_no_submodules），v1 接受，spec 不承诺含子模块项目可删。
- [分支孤儿] → `worktree remove` 不删分支，v1 刻意保数据；用户可
  自行清理，spec 不承诺分支删除。
- [手删目录遗留 prunable 条目] → 列表按 git 输出如实呈现，prune
  清理入口后置。
- [快照现算每次跑 `git worktree list`] → N 小可接受；若 load/get_tree
  频繁路径实测变慢，缓存为后续优化（不改行为，不需 spec）。

## Migration Plan

纯新增能力，无数据迁移、无 breaking：既有 solutions 文件、层级、
消息 kind 全部不变（新增一个完成事件 kind 为追加）。回滚 = revert
提交即可，无状态残留（worktree 目录是唯一持久副作用，回滚后用户
可自行删除或留用 —— 它们是合法 git worktree）。

## Open Questions

以下可后置且不改 spec/方案/任务分解：

1. **pull 语义**：ff-only 还是允许 merge？冲突/无 upstream 时的
   错误文案归类？（当前 spec 只承诺「跑」与失败回执，不承诺策略）
2. **create 表单参数**：ref 从 HEAD 还是可选分支？参数集与默认值
   （solution-overview 留了 `args` 通道，表单 UI 属后续）。
3. **目录名撞车消歧细则**：`<project>-<ref>` + 短 hash 的具体规则。
4. **prune 清理入口**：prunable 条目要不要列表徽标 + 一键 prune。
5. **删除是否顺带删分支**：v1 保数据（D6 风险表已记），用户若要求
   可加 `branch -d` 选项。
