# Tasks

## 1. 合同层

- [x] 1.1 `crates/contracts/src/shared.rs`：新增 `ActionChild { label, node }`，
      `ActionDesc` 加 `children: Arc<[ActionChild]>`；全部 7 处构造点与文档示例
      补空 `children`，asset 行渲染不改；验证：`cargo check --workspace` 通过、
      contracts 既有测试不红
- [x] 1.2 solution-service 合同：`GitOpFinished` 加
      `statuses: Vec<RepoStatus>`（`RepoStatus{path,state}` / `Clean|Dirty|Conflict`），
      构造点补字段；验证：`cargo check -p solution-service` 通过
- [x] 1.3 solution-service 合同：`action_label(id) -> Option<&'static str>` 静态表
      （7 个已声明 id 全覆盖）；验证：单测断言 7 个 label 命中、未知 id 返回 None

## 2. git 模块 merge helpers

- [x] 2.1 `git/mod.rs` 新增异步出口 `head_sha` / `is_ancestor` / `ahead_count` /
      `merge(cwd, sha, ff_only)`（薄包既有 `run_async`，同步核心保持模块私有）；
      验证：单测用临时真仓 fixture 断言 —— ff 合成功、冲突时退出码+输出含
      `CONFLICT`、ancestor/count 数值正确；`cargo test -p solution-service git::` 绿

## 3. 服务端：描述符 + 两个 handler

- [x] 3.1 `project_actions(&worktrees)` 改签名：worktree 层非空时追加
      "从 worktree 同步" 父描述符（children = 每 worktree 一条，label=显示名、
      node=裸路径）；`worktree_actions()` 追加 "同步 Project 代码"；
      更新既有契约断言（project 3→4、worktree 1→2），新增：空层无父、
      children 数量/label/node 正确；验证：`cargo test -p solution-service contract` 绿
- [x] 3.2 `worktree.sync_from_project` handler（对齐 offload 后 delete 形态：
      handle 仅节点格式检查 → 秒回 accepted → 池内属主查找/is_git/tracked-clean
      门/merge）；冲突 = 退出码+`CONFLICT` → detail 列文件、statuses 记 Conflict；
      验证：服务测试 4 例 —— 干净合并成功（statuses=Clean、detail 含 merge 摘要）、
      脏拒绝（statuses=Dirty、仓库零改动）、冲突（detail 列文件+Conflict）、
      非 git 拒绝
- [x] 3.3 `project.sync_from_worktree` handler：四门定序
      （project 脏 → wt 脏 → 不含 → 领先=0）+ `git merge --ff-only`；
      两仓 statuses 随事件回传；验证：服务测试 5 例 —— ahead 成功 ff、
      behind/分叉 → 文案指回 worktree 侧同步、equal → 无操作拒绝、
      任一侧脏 → 拒绝且该侧 statuses=Dirty、非 git 拒绝
- [x] 3.4 既有 pull/push/create/delete 的 outcome 补 statuses（门禁判出的仓、
      成功执行的仓）；验证：既有 outcome 测试更新并断言 statuses，全绿

## 4. 视图层：子菜单 + 徽标 + 通知文案

- [x] 4.1 `build_project_row` 按 `desc.children` 分支渲染：空 → 既有平铺路径、
      非空 → `PopupMenuItem::submenu`，子项点击 invoke
      `(node=child.node, action_id=父id)`；验证：视图测试 —— 带 children 的
      快照渲染出子菜单、子项点击发出正确 invoke（对齐
      `create_worktree_menu_builds_default_name` 的既有形态）
- [x] 4.2 徽标：panel/app 层 `path → state` 映射，消费 `GitOpFinished.statuses`，
      project/worktree 行渲染四态（灰未知/绿 Clean/黄 Dirty/红 Conflict，
      语义 token 取自 theme）；验证：视图测试 —— 事件喂入后涉及行变色、
      未涉及行保持原态、刷新快照不丢徽标
- [x] 4.3 `service_layer` `KIND_GIT_OP_FINISHED` 文案经 `action_label` 映射
      （未知 id 回退裸 id）；同步更新既有 raw-id 文案断言；验证：
      `cargo test -p balabala-agent-gui`（或对应 lib 测试）断言
      "推送失败: …" 式文案 + 回退分支两例

## 5. 集成验证

- [x] 5.1 全 workspace 测试与存量红基线（bb92b0d 记录的 20 个环境失败）
      diff —— 断言 0 新增失败；`cargo fmt` 仅保本次改动区域干净
- [x] 5.2 离载回归：新增 git 调用（merge/ancestor/count）线程 ID 断言测试
      （对齐 offload 的 latched 模式）；变异检查 —— 临时移除某处
      `spawn_blocking` 包装该测试变红，恢复后绿

## 6. Windows 真机冒烟（用户执行，结果记录于本 tasks）

- [x] 6.1 双向同步真机验证：project 行子菜单列出 worktree 并可同步（ff 成功
      notice + 徽标转绿）；worktree 行 "同步 Project 代码" 在分叉仓合并、
      人为制造冲突可见文件列表通知 + 行徽标红；四门各触发一次拒绝文案
      （含 "先在 worktree 同步" 指引）；既有四动作 finish 文案显示中文 label；
      验证：用户目视 + 记录于本 tasks
      **验收记录（2026-11-29, Windows, 用户真机目视）**：整体冒烟无问题
      （用户：“整体冒烟没什么问题”）。[notice] 实测印证：① label 映射
      生效 —— `创建 worktree完成: …` / `同步 Project 代码完成: …` /
      `从 worktree 同步失败: …` 全中文，裸 id 已消失；② D1 成功带 merge
      摘要（`Already up to date.`）；③ D2 门1 拒绝点名 tracked 改动并
      指回提交。子菜单渲染、四态徽标（初始灰/转绿/冲突红）、冲突文件
      列表、四门其余拒绝按整体确认；日志仅常态 term-rate 无 panic

## Workflow follow-up

- 全量复核通过后：`openspec archive "solution-git-sync" --yes` 同步 delta
  到主 specs，`openspec validate --specs` 复核（16/16、警告与基线一致），
  提交并推送。
