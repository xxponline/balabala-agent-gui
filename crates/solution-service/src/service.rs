//! `SolutionService`：Solution/Project 域的进程内总线服务（design D1/D7/D8）。
//!
//! 状态单份归属服务端：`items` = 已加载的 solution 配置真值；项目树/交树
//! 快照由 [`Self::snapshot`] 现算（is_git 解析 + 注册门控）。WorkTree 管理
//! 后续挂接（本 change 只留口子，不建空壳）。

use std::path::PathBuf;
use std::sync::Arc;

use crate::contract::{
    ACTION_CONFIGURE_GIT, ACTION_CONFIGURE_REGISTER, ACTION_CREATE, ACTION_CREATE_WORKTREE,
    ACTION_DELETE_WORKTREE, ACTION_PULL, ACTION_PUSH, ACTION_SYNC_PROJECT_FROM_WORKTREE,
    ACTION_SYNC_WORKTREE_FROM_PROJECT, ActionInvoke, ConfigureReq, CreateReq, CreateWorktreeReq,
    GetTreeReq, GitOpFinished, KIND_ACTION, KIND_GET_TREE, KIND_GIT_OP_FINISHED, KIND_LOAD,
    KIND_OP_REPLY, KIND_TREE_CHANGED, KIND_WATCH, LoadReq, OpError, OpResult, ProjectNode,
    RepoState, RepoStatus, SolutionItem, SolutionNode, TreeChanged, TreeSnapshot, WORKTREES_DIR,
    WatchReq, WorktreeNode, parse_project_node, parse_solution_node, project_actions,
    solution_actions, worktree_actions,
};
use crate::loader;
use crate::valid_checker;
use service_core::{
    Address, Envelope, Payload, Route, Service, ServiceCtx, ServiceFuture, TaskFuture,
};

/// 未绑定工作区就发 create/configure 的回执文案（design D5）。
///
/// 正常链路（Launcher → load → 用户操作）走不到; 真到了就是调用顺序 bug —
/// 以红条暴露, 不 panic、不静默丢弃。
const NO_BOUND_WORKSPACE: &str = "no workspace bound — load the workspace first";

/// 属主查找（池内, change solution-git-sync D1/D2 与 delete 同款）：
/// 遍历 git 项目的 worktree 层, canonicalize 匹配 → 属主 project root;
/// 找不到 → `None`（无此 worktree, 或 solution 关了 git 管理）。
async fn find_worktree_owner(items: &[SolutionItem], path: &std::path::Path) -> Option<PathBuf> {
    for item in items {
        let desc = loader::expand_item(item);
        if !desc.is_git {
            continue;
        }
        let Ok(entries) = crate::git::worktree::list_async(&desc.root).await else {
            continue;
        };
        let canon =
            |p: &std::path::Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        if entries.iter().any(|e| canon(&e.path) == canon(path)) {
            return Some(desc.root.clone());
        }
    }
    None
}

/// merge 成功摘要（进 notice detail）: 取带语义的摘要行 ——
/// `Fast-forward`（ff 时首行是 `Updating a..b`, 摘要在次行）/
/// `Merge made by ...` / `Already up to date.`；无匹配退首个非空行。
fn merge_summary(out: &crate::git::GitOutput) -> String {
    let lines: Vec<&str> = out
        .stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    lines
        .iter()
        .find(|l| {
            l.starts_with("Fast-forward")
                || l.starts_with("Merge made")
                || l.starts_with("Already up to date")
        })
        .or_else(|| lines.first())
        .copied()
        .unwrap_or("已合并")
        .to_string()
}

/// 冲突详情（进 notice detail）: 提取 `Merge conflict in <file>` 文件
/// 列表 + 未完成合并引导; 提取不到 → 退 git 原话尾部。
fn conflict_detail(out: &crate::git::GitOutput) -> String {
    let combined = format!("{}{}", out.stdout, out.stderr);
    let files: Vec<&str> = combined
        .lines()
        .filter_map(|l| l.split("Merge conflict in ").nth(1))
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    if files.is_empty() {
        let tail = crate::git::GitError::Failed {
            code: out.code,
            stderr: out.stderr.clone(),
        }
        .to_string();
        format!("合并冲突, 仓库处于未完成合并状态, 请解决后提交: {tail}")
    } else {
        format!(
            "合并冲突 — {}, 仓库处于未完成合并状态, 请解决后提交",
            files.join(", ")
        )
    }
}

/// Solution/Project 域服务（driver 单线程顺序 dispatch，同步处理 + 同步回执，
/// 对齐 `AgentService` 形态）。
pub struct SolutionService {
    ctx: ServiceCtx,
    /// 状态单份（design D7）：solution items 的真值（视图经快照镜像读取）。
    items: Vec<SolutionItem>,
    /// 已绑定工作区的目录 — create/configure 落盘目标（design D5）。
    ///
    /// 由 `on_load` **成功**时写入; load 失败不碰（spec: 失败不覆盖既有
    /// 状态）。请求因此不再各自重申 `workspace_dir`。
    workspace_dir: Option<PathBuf>,
    /// 树变更订阅者（watch 握手记录；单订阅者，对齐 AgentService）。
    watcher: Option<service_core::InstanceId>,
}

impl SolutionService {
    pub fn new(ctx: ServiceCtx) -> Self {
        Self {
            ctx,
            items: Vec::new(),
            workspace_dir: None,
            watcher: None,
        }
    }

    /// 由当前状态现算快照（spec: Asset registration flag gates project
    /// handoff —— `handoff` 只含注册开关开启的 solution 的 project;
    /// `tree` 为全量集, 注册关闭的项目仍属于 solution 的 project 集）。
    ///
    /// **`hierarchy` 是唯一展开数据**（change solution-overview D1）: 这里
    /// 只算嵌套结构, 平表与交树由 `TreeSnapshot::tree()` /
    /// `handoff()` 派生 — 不再在构造点手工维护两份投影。
    ///
    /// 节点上的 `actions` 由服务按 item 当前配置现算（design D3）—
    /// 描述符搭快照的车下传, 右键菜单因此零往返。
    pub async fn snapshot(&self) -> TreeSnapshot {
        let mut nodes = Vec::with_capacity(self.items.len());
        for item in &self.items {
            // v1 `simple`: 1 item → 1 project (spec: Solution type
            // registry resolves items to projects)
            // worktree 层 = git 元数据现算 (task 4.3 真源, design D4):
            // `git worktree list --porcelain` 可见应用外注册的条目
            // (异步出口 → 阻塞池, spec: Git work does not delay
            // other services)
            let worktrees = Self::worktree_layer(item).await;
            // 描述符随层内容现算 (solution-git-sync D3: children 同源)
            let actions = project_actions(&worktrees);
            let projects = Arc::from(vec![ProjectNode {
                desc: loader::expand_item(item),
                worktrees,
                actions,
            }]);
            nodes.push(SolutionNode {
                item: item.clone(),
                projects,
                actions: solution_actions(item),
            });
        }
        let hierarchy: Arc<[SolutionNode]> = nodes.into();
        TreeSnapshot {
            solutions: self.items.clone().into(),
            hierarchy,
        }
    }

    /// 一个 project 的 worktree 层（hierarchy 第三层）：真源 = git
    /// `worktree list --porcelain`（spec: Worktree list comes from git
    /// metadata）。主仓自身条目（path == project root）滤掉；非 git
    /// 项目或 git 调用失败 → 空层（不阻断快照）。子进程走异步出口
    /// （阻塞池）—— 列表不占单 worker（change solution-git-offload D1）。
    async fn worktree_layer(item: &SolutionItem) -> Arc<[WorktreeNode]> {
        let desc = loader::expand_item(item);
        if !desc.is_git {
            return Arc::from([]);
        }
        let Ok(entries) = crate::git::worktree::list_async(&desc.root).await else {
            return Arc::from([]);
        };
        let canon = |p: &std::path::Path| {
            std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
        };
        let root = canon(&desc.root);
        entries
            .into_iter()
            .filter(|e| canon(&e.path) != root) // 滤主仓自身
            .map(|e| WorktreeNode {
                path: e.path,
                branch: e.branch,
                actions: worktree_actions(),
            })
            .collect::<Vec<_>>()
            .into()
    }

    /// 信封分发（driver 顺序上下文；未知 kind 拒收落死信，对齐 AgentService）。
    /// 异步 —— git 子进程离载后 handler 可 await（spec: Git work does
    /// not delay other services）。
    async fn dispatch(&mut self, env: Envelope) {
        match env.kind {
            KIND_LOAD => self.on_load(&env).await,
            KIND_GET_TREE => self.on_get_tree(&env).await,
            KIND_ACTION => self.on_action(&env).await,
            KIND_WATCH => self.on_watch(&env).await,
            // 后台 worktree 完成事件回投自己 (create/delete 落地)
            KIND_GIT_OP_FINISHED => self.on_git_op_result(&env).await,
            _ => {
                let _ = self.ctx.messaging.reject(&env);
            }
        }
    }

    /// 控制回执（仅 ask：`corr` 存在才回，对齐 `AgentService::reply_to`）。
    fn reply_to(&self, env: &Envelope, result: OpResult) {
        if env.corr.is_some() {
            let _ = self
                .ctx
                .messaging
                .reply(env, KIND_OP_REPLY, Payload::new(result));
        }
    }

    /// 树变更事件（推 watcher；事件自带快照）。
    fn emit(&self, snap: TreeSnapshot) {
        let Some(watcher) = self.watcher else {
            return;
        };
        let _ = self.ctx.messaging.tell(Envelope::new(
            Route::Instance(watcher),
            Address::instance(self.ctx.instance),
            KIND_TREE_CHANGED,
            Payload::new(TreeChanged(snap)),
        ));
    }

    /// `load`：加载边界统一入口（启动 restore / 手动切换 / 显式 reload）。
    /// 失败只有结构化 reason，**不覆盖既有状态、不缓存部分树**（spec）。
    async fn on_load(&mut self, env: &Envelope) {
        let Ok(req) = env.downcast::<LoadReq>() else {
            let _ = self.ctx.messaging.reject(env);
            return;
        };
        match loader::load_items(&req.workspace_dir) {
            Err(e) => self.reply_to(env, Err(OpError::Load(e))),
            Ok(items) => {
                // 绑定写入与状态更新同支 (design D5): 失败支不碰, 对齐
                // spec "失败不覆盖既有状态"。
                self.workspace_dir = Some(req.workspace_dir.clone());
                let changed = items != self.items;
                self.items = items;
                let snap = self.snapshot().await;
                self.reply_to(env, Ok(snap.clone()));
                if changed {
                    self.emit(snap);
                }
            }
        }
    }

    /// `get_tree`：镜像补拉（无状态变更）。
    async fn on_get_tree(&self, env: &Envelope) {
        let Ok(_) = env.downcast::<GetTreeReq>() else {
            let _ = self.ctx.messaging.reject(env);
            return;
        };
        self.reply_to(env, Ok(self.snapshot().await));
    }

    /// `solution.action` — **写入口单点**（design D6）。
    ///
    /// 分三层, 错误归属各不相同：
    /// 1. 载荷 downcast 失败 → `reject`（契约 bug, `TypeMismatch` 同类）
    /// 2. `action_id` 服务未声明 → **立即回** `OpError::UnknownAction`
    ///    （实施期定案 C — 不 reject, 见 design D6）
    /// 3. `node` 格式坏 → `reject`；前缀对但无此 solution → `NoSuch`
    ///    （那两条在 `on_create` / `on_configure` 内）
    async fn on_action(&mut self, env: &Envelope) {
        let Ok(req) = env.downcast::<ActionInvoke>() else {
            let _ = self.ctx.messaging.reject(env);
            return;
        };
        match req.action_id.as_str() {
            ACTION_CREATE => self.on_create(env, &req).await,
            ACTION_CONFIGURE_REGISTER | ACTION_CONFIGURE_GIT => self.on_configure(env, &req).await,
            ACTION_PULL | ACTION_PUSH => self.on_git_pull_push(env, &req).await,
            ACTION_CREATE_WORKTREE => self.on_create_worktree(env, &req).await,
            ACTION_DELETE_WORKTREE => self.on_delete_worktree(env, &req).await,
            ACTION_SYNC_WORKTREE_FROM_PROJECT => {
                self.on_worktree_sync_from_project(env, &req).await
            }
            ACTION_SYNC_PROJECT_FROM_WORKTREE => {
                self.on_project_sync_from_worktree(env, &req).await
            }
            other => {
                self.reply_to(env, Err(OpError::UnknownAction(other.to_string())));
            }
        }
    }

    /// pull/push 门控（change solution-git-worktree 3.1, design D2）：
    /// 双门 —— is_git（解析产出, 同 snapshot 口径）留 handle 秒判
    /// （`.git` 探测 = fs metadata）；tracked-clean（大仓 `git status`
    /// 可达秒级）与执行一起挪进后台阻塞池（change solution-git-offload
    /// D2）：秒回 accepted（纯传输应答, 5s 内销账）, **拒绝与执行结果
    /// 走完成事件 notice** —— 执行好后再反馈, 仓库始终零改动。
    async fn on_git_pull_push(&mut self, env: &Envelope, invoke: &ActionInvoke) {
        // 目标单源 = node (project://<root>); 格式坏 = 契约 bug → reject
        let Some(root) = parse_project_node(invoke.node.as_deref()) else {
            let _ = self.ctx.messaging.reject(env);
            return;
        };
        // v1: 1 item → 1 project, project root == item path
        let Some(item) = self.items.iter().find(|i| i.path() == root.as_path()) else {
            self.reply_to(env, Err(OpError::NoSuch(root)));
            return;
        };
        // 门1: is_git (非 git 项目 / solution 关了 git 管理 → 拒)
        if !loader::expand_item(item).is_git {
            self.reply_to(env, Err(OpError::Git(format!(
                "{} 不是 git 管理的项目",
                root.display()
            ))));
            return;
        }
        // 门2 + 执行全在后台 (design D4 门到执行背靠背); 回执在 spawn
        // 之前同步发出, 顺序有保证 (design D1: 5s 内销账, 无死信)。
        self.reply_to(env, Ok(self.snapshot().await));
        self.spawn_git_op(&root, invoke.action_id.as_str());
    }

    /// 后台跑「tracked-clean 门控 + pull/push」，完成发**无 corr** 的
    /// `KIND_GIT_OP_FINISHED`（design D1）：原 corr 已在 accepted 回执
    /// 销账, 带 corr 迟到 = 死信；事件直发 watch 订阅者（GUI pump 按
    /// action_id 出 notice）。
    ///
    /// **git 子进程全在阻塞池**（change solution-git-offload D1/D2）：
    /// 大仓 status 门控与网络 pull/push 都不得占单 worker —— 门控拒绝
    /// 与执行失败走同一条完成事件链路（spec: outcome SHALL surface as
    /// a notice）。**操作总是执行** —— 无订阅者只是事件无人收, 不得
    /// 反向阻断 git 本身。
    fn spawn_git_op(&self, root: &std::path::Path, action_id: &str) {
        let messaging = Arc::clone(&self.ctx.messaging);
        let instance = self.ctx.instance;
        let watcher = self.watcher;
        let action_id = action_id.to_string();
        let target = root.display().to_string();
        let root = root.to_path_buf();
        let fut: TaskFuture = Box::pin(async move {
            // 门控 + 执行 = 同一后台任务背靠背 (design D4), 子进程各自
            // 经异步出口落阻塞池 —— 大仓 status 与网络 pull/push 都不占
            // 单 worker (D1/D2)
            // 门2: tracked-clean (脏 → 拒, 仓库零改动); 状态随事件回传
            // (solution-git-sync D4: 门判出/执行后的仓 → 行徽标)
            let clean = || {
                vec![RepoStatus {
                    path: target.clone(),
                    state: RepoState::Clean,
                }]
            };
            let (ok, detail, statuses) = match crate::git::is_tracked_clean_async(&root).await {
                Ok(false) => (
                    false,
                    format!("{target} 有未提交的 tracked 改动, 先提交再拉取/推送"),
                    vec![RepoStatus {
                        path: target.clone(),
                        state: RepoState::Dirty,
                    }],
                ),
                Err(e) => (false, e.to_string(), Vec::new()),
                Ok(true) => {
                    let args: &[&str] =
                        if action_id == ACTION_PULL { &["pull"] } else { &["push"] };
                    match crate::git::run_ok_async(args, &root).await {
                        Ok(_) => (true, String::new(), clean()),
                        Err(e) => {
                            // pull 合并冲突 → Conflict（区别于门判的 Clean）
                            let st = if e.to_string().contains("CONFLICT") {
                                vec![RepoStatus {
                                    path: target.clone(),
                                    state: RepoState::Conflict,
                                }]
                            } else {
                                clean()
                            };
                            (false, e.to_string(), st) // stderr 尾部摘要 (design D6)
                        }
                    }
                }
            };
            let Some(watcher) = watcher else {
                return; // 无订阅者: 操作已完, 事件无人收
            };
            let _ = messaging.tell(Envelope::new(
                Route::Instance(watcher),
                Address::instance(instance),
                KIND_GIT_OP_FINISHED,
                Payload::new(GitOpFinished {
                    action_id,
                    target,
                    ok,
                    detail,
                    statuses,
                }),
            ));
        });
        self.ctx.messaging.spawn_task(fut);
    }

    /// `worktree.create`（4.1）：目标 = `project://<root>`，参数 =
    /// `CreateWorktreeReq{name}`；目录落**已绑定**工作区的
    /// `.balabala.worktrees/`（design D5 绑定复用 + D4 点前缀）。
    /// **真异步**（change solution-git-offload D3）：廉价校验（node/
    /// is_git/args/名字消毒/建目录）留 handle 秒拒；秒回 accepted
    /// （操作前快照），`worktree add` 在阻塞池跑，成功 → 完成事件回投
    /// 自己 → `TreeChanged`；失败 → 完成事件 notice（同 pull/push 链路）。
    async fn on_create_worktree(&mut self, env: &Envelope, invoke: &ActionInvoke) {
        let Some(root) = parse_project_node(invoke.node.as_deref()) else {
            let _ = self.ctx.messaging.reject(env);
            return;
        };
        let Some(item) = self.items.iter().find(|i| i.path() == root.as_path()) else {
            self.reply_to(env, Err(OpError::NoSuch(root)));
            return;
        };
        if !loader::expand_item(item).is_git {
            self.reply_to(env, Err(OpError::Git(format!(
                "{} 不是 git 管理的项目",
                root.display()
            ))));
            return;
        }
        // 参数: 缺失/类型不对 = 契约 bug → reject (同 on_create)
        let Some(req) = invoke.args.as_ref().and_then(|a| a.downcast::<CreateWorktreeReq>())
        else {
            let _ = self.ctx.messaging.reject(env);
            return;
        };
        // 名字消毒 = 纯 CPU → 留 handle 秒拒 (`..` 等非法名不等后台);
        // bg 的 `add` 会再消毒一次 (幂等, 同一份实现)。
        let sanitized = match crate::git::worktree::sanitize_name(&req.name) {
            Ok(s) => s,
            Err(e) => {
                self.reply_to(env, Err(OpError::Git(e.to_string())));
                return;
            }
        };
        // 目标目录单源 = 绑定工作区 (design D5, 同 create/configure)
        let Some(ws_dir) = self.workspace_dir.clone() else {
            self.reply_to(env, Err(OpError::Persist(NO_BOUND_WORKSPACE.into())));
            return;
        };
        let parent = ws_dir.join(WORKTREES_DIR);
        if let Err(e) = std::fs::create_dir_all(&parent) {
            self.reply_to(env, Err(OpError::Git(format!(
                "无法创建 {}: {e}",
                parent.display()
            ))));
            return;
        }
        // 秒回 accepted = 操作前快照 (层级真变由完成后的 TreeChanged 送达)
        let snap = self.snapshot().await;
        self.reply_to(env, Ok(snap));
        let target = parent.join(&sanitized).display().to_string();
        let name = req.name.clone();
        let messaging = Arc::clone(&self.ctx.messaging);
        let instance = self.ctx.instance;
        let fut: TaskFuture = Box::pin(async move {
            // `worktree add` 经异步出口落阻塞池 (D1), 完成回投自己
            let outcome = crate::git::worktree::add_async(&root, &parent, &name).await;
            // 成功 → 新 worktree 检出即净, 回传其状态（徽标新行）; 失败 → 未定
            let (ok, detail, statuses) = match outcome {
                Ok(_) => (
                    true,
                    String::new(),
                    vec![RepoStatus {
                        path: target.clone(),
                        state: RepoState::Clean,
                    }],
                ),
                Err(e) => (false, e.to_string(), Vec::new()),
            };
            let _ = messaging.tell(Envelope::new(
                Route::Instance(instance),
                Address::instance(instance),
                KIND_GIT_OP_FINISHED,
                Payload::new(GitOpFinished {
                    action_id: ACTION_CREATE_WORKTREE.to_string(),
                    target,
                    ok,
                    detail,
                    statuses,
                }),
            ));
        });
        self.ctx.messaging.spawn_task(fut);
    }

    /// `worktree.delete`（4.2）：node = worktree 裸路径（solution-overview
    /// D2: worktree 行用裸路径）。**服务端只跑 git 自己的门** ——
    /// `worktree remove` 无 `--force`，git 内置 `check_clean_worktree`
    /// 当裁判（design D2：先查后删原子, 脏则零字节不动）。会话门在校验1
    /// 归 view（design D3 路线 A）。**真异步**（change
    /// solution-git-offload D3）：秒回 accepted（操作前快照），属主查找
    /// 与 remove 都在阻塞池；成功 → `TreeChanged`，失败/无此 worktree →
    /// 完成事件 notice。
    async fn on_delete_worktree(&mut self, env: &Envelope, invoke: &ActionInvoke) {
        // 裸路径节点 (无前缀): 缺失 = 契约 bug → reject
        let Some(path) = invoke.node.as_deref().map(std::path::PathBuf::from) else {
            let _ = self.ctx.messaging.reject(env);
            return;
        };
        // 秒回 accepted = 操作前快照 (删除成功由 TreeChanged 送达)
        let snap = self.snapshot().await;
        self.reply_to(env, Ok(snap));
        // 属主查找 = `git worktree list` 子进程 → 经异步出口落阻塞池
        // (D1); items 克隆随任务走 (单线程状态纪律: 快照仍在 handle
        // 重建)。
        let items = self.items.clone();
        let messaging = Arc::clone(&self.ctx.messaging);
        let instance = self.ctx.instance;
        let fut: TaskFuture = Box::pin(async move {
            let mut owner: Option<std::path::PathBuf> = None;
            for item in &items {
                let desc = loader::expand_item(item);
                if !desc.is_git {
                    continue;
                }
                let Ok(entries) = crate::git::worktree::list_async(&desc.root).await else {
                    continue;
                };
                let canon = |p: &std::path::Path| {
                    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
                };
                if entries.iter().any(|e| canon(&e.path) == canon(&path)) {
                    owner = Some(desc.root.clone());
                    break;
                }
            }
            let outcome = match owner {
                Some(repo_root) => crate::git::worktree::remove_async(&repo_root, &path).await,
                None => Err(crate::git::GitError::Failed {
                    code: -1,
                    stderr: format!("no such worktree: {}", path.display()),
                }),
            };
            let (ok, detail, statuses) = match outcome {
                // 仓已删 → 无处徽标; 拒 → **不回传状态** —— git 删除门
                // 计 untracked, 与 tracked-clean 口径不一致, 重推导会错判
                // （拒绝原话已直达 notice, 徽标非 spec 要求）
                Ok(()) => (true, String::new(), Vec::new()),
                Err(e) => (false, e.to_string(), Vec::new()),
            };
            let _ = messaging.tell(Envelope::new(
                Route::Instance(instance),
                Address::instance(instance),
                KIND_GIT_OP_FINISHED,
                Payload::new(GitOpFinished {
                    action_id: ACTION_DELETE_WORKTREE.to_string(),
                    target: path.display().to_string(),
                    ok,
                    detail,
                    statuses,
                }),
            ));
        });
        self.ctx.messaging.spawn_task(fut);
    }

    /// `worktree.sync_from_project`（change solution-git-sync D1）：node =
    /// worktree 裸路径。把 project 当前 HEAD 合入该 worktree —— **真合并**
    /// （可冲突）。门只看 worktree 侧 tracked-clean（合并动的是 worktree
    /// 的树, project 工作树不被改写 → 不门）。形态同 offload 后的 delete：
    /// handle 仅节点格式检查 → 秒回 accepted → 池内属主查找/门/merge →
    /// `KIND_GIT_OP_FINISHED` 直发订阅者（层级不变, 无 TreeChanged）。
    async fn on_worktree_sync_from_project(&mut self, env: &Envelope, invoke: &ActionInvoke) {
        let Some(path) = invoke.node.as_deref().map(PathBuf::from) else {
            let _ = self.ctx.messaging.reject(env);
            return;
        };
        let snap = self.snapshot().await;
        self.reply_to(env, Ok(snap));
        let items = self.items.clone();
        let messaging = Arc::clone(&self.ctx.messaging);
        let instance = self.ctx.instance;
        let watcher = self.watcher;
        let target = path.display().to_string();
        let fut: TaskFuture = Box::pin(async move {
            let (ok, detail, statuses): (bool, String, Vec<RepoStatus>) = async {
                let Some(repo_root) = find_worktree_owner(&items, &path).await else {
                    return (false, format!("no such worktree: {target}"), Vec::new());
                };
                // 门: worktree tracked-clean（拒绝 → 该行徽标 Dirty）
                match crate::git::is_tracked_clean_async(&path).await {
                    Ok(false) => {
                        return (
                            false,
                            format!("{target} 有未提交的 tracked 改动, 先提交再同步"),
                            vec![RepoStatus {
                                path: target.clone(),
                                state: RepoState::Dirty,
                            }],
                        );
                    }
                    Err(e) => return (false, e.to_string(), Vec::new()),
                    Ok(true) => {}
                }
                // project HEAD → merge 入 worktree（真合并, 可冲突）
                let sha = match crate::git::head_sha(&repo_root).await {
                    Ok(sha) => sha,
                    Err(e) => return (false, e.to_string(), Vec::new()),
                };
                let out = match crate::git::merge(&path, &sha, false).await {
                    Ok(out) => out,
                    Err(e) => return (false, e.to_string(), Vec::new()),
                };
                if out.code == 0 {
                    (
                        true,
                        merge_summary(&out),
                        vec![RepoStatus {
                            path: target.clone(),
                            state: RepoState::Clean,
                        }],
                    )
                } else if format!("{}{}", out.stdout, out.stderr).contains("CONFLICT") {
                    (
                        false,
                        conflict_detail(&out),
                        vec![RepoStatus {
                            path: target.clone(),
                            state: RepoState::Conflict,
                        }],
                    )
                } else {
                    (
                        false,
                        crate::git::GitError::Failed {
                            code: out.code,
                            stderr: out.stderr,
                        }
                        .to_string(),
                        Vec::new(),
                    )
                }
            }
            .await;
            let Some(watcher) = watcher else {
                return; // 无订阅者: 操作已完, 事件无人收
            };
            let _ = messaging.tell(Envelope::new(
                Route::Instance(watcher),
                Address::instance(instance),
                KIND_GIT_OP_FINISHED,
                Payload::new(GitOpFinished {
                    action_id: ACTION_SYNC_WORKTREE_FROM_PROJECT.to_string(),
                    target,
                    ok,
                    detail,
                    statuses,
                }),
            ));
        });
        self.ctx.messaging.spawn_task(fut);
    }

    /// `project.sync_from_worktree`（change solution-git-sync D2）：node =
    /// worktree 裸路径（子菜单 child 回传）。把 worktree HEAD **快进**回
    /// project —— 四门定序（design D1）：project 脏 → wt 脏 → 不含 →
    /// 领先=0，全过才 `merge --ff-only`（contains+ahead 已保证快进,
    /// `--ff-only` 防 HEAD 竞动时静默造 merge commit）。两仓状态随完成
    /// 事件回传（design D4）；层级不变 → 直发订阅者, 无 TreeChanged。
    async fn on_project_sync_from_worktree(&mut self, env: &Envelope, invoke: &ActionInvoke) {
        let Some(path) = invoke.node.as_deref().map(PathBuf::from) else {
            let _ = self.ctx.messaging.reject(env);
            return;
        };
        let snap = self.snapshot().await;
        self.reply_to(env, Ok(snap));
        let items = self.items.clone();
        let messaging = Arc::clone(&self.ctx.messaging);
        let instance = self.ctx.instance;
        let watcher = self.watcher;
        let action_id = ACTION_SYNC_PROJECT_FROM_WORKTREE.to_string();
        let fut: TaskFuture = Box::pin(async move {
            let owner = find_worktree_owner(&items, &path).await;
            // notice 展示目标: 属主 project root; 无属主 → 退 worktree 路径
            let target = owner
                .as_ref()
                .map(|r| r.display().to_string())
                .unwrap_or_else(|| path.display().to_string());
            let (ok, detail, statuses): (bool, String, Vec<RepoStatus>) = async {
                let Some(repo_root) = owner else {
                    return (
                        false,
                        format!("no such worktree: {}", path.display()),
                        Vec::new(),
                    );
                };
                // 门1: project tracked-clean（拒绝 → project 行徽标 Dirty）
                match crate::git::is_tracked_clean_async(&repo_root).await {
                    Ok(false) => {
                        let msg = format!(
                            "{} 有未提交的 tracked 改动, 先提交再同步",
                            repo_root.display()
                        );
                        return (
                            false,
                            msg,
                            vec![RepoStatus {
                                path: repo_root.display().to_string(),
                                state: RepoState::Dirty,
                            }],
                        );
                    }
                    Err(e) => return (false, e.to_string(), Vec::new()),
                    Ok(true) => {}
                }
                // 门2: worktree tracked-clean（拒绝 → wt 行徽标 Dirty）
                match crate::git::is_tracked_clean_async(&path).await {
                    Ok(false) => {
                        let msg =
                            format!("{} 有未提交的 tracked 改动, 先提交再同步", path.display());
                        return (
                            false,
                            msg,
                            vec![
                                RepoStatus {
                                    path: repo_root.display().to_string(),
                                    state: RepoState::Clean,
                                },
                                RepoStatus {
                                    path: path.display().to_string(),
                                    state: RepoState::Dirty,
                                },
                            ],
                        );
                    }
                    Err(e) => return (false, e.to_string(), Vec::new()),
                    Ok(true) => {}
                }
                // 双门已过: 后续逻辑类拒绝也回传这对 Clean 状态
                let clean_both = vec![
                    RepoStatus {
                        path: repo_root.display().to_string(),
                        state: RepoState::Clean,
                    },
                    RepoStatus {
                        path: path.display().to_string(),
                        state: RepoState::Clean,
                    },
                ];
                // 两侧 HEAD
                let p_head = match crate::git::head_sha(&repo_root).await {
                    Ok(s) => s,
                    Err(e) => return (false, e.to_string(), clean_both),
                };
                let w_head = match crate::git::head_sha(&path).await {
                    Ok(s) => s,
                    Err(e) => return (false, e.to_string(), clean_both),
                };
                // 门3: wt 包含 project HEAD（不分叉/未落后 → 指回 D1）
                match crate::git::is_ancestor(&p_head, &w_head, &repo_root).await {
                    Ok(false) => {
                        return (
                            false,
                            "worktree 未包含 Project 最新提交, 先在 worktree 同步 Project 代码"
                                .to_string(),
                            clean_both,
                        );
                    }
                    Err(e) => return (false, e.to_string(), clean_both),
                    Ok(true) => {}
                }
                // 门4: 领先 >=1（相等 = 无操作）
                match crate::git::ahead_count(&p_head, &w_head, &repo_root).await {
                    Ok(0) => {
                        return (
                            false,
                            "worktree 与 Project 同步点一致, 无可同步的新提交".to_string(),
                            clean_both,
                        );
                    }
                    Ok(_) => {}
                    Err(e) => return (false, e.to_string(), clean_both),
                }
                // 执行: ff-only 快进
                match crate::git::merge(&repo_root, &w_head, true).await {
                    Err(e) => (false, e.to_string(), clean_both),
                    Ok(out) if out.code == 0 => (true, merge_summary(&out), clean_both),
                    Ok(out) => (
                        false,
                        crate::git::GitError::Failed {
                            code: out.code,
                            stderr: out.stderr,
                        }
                        .to_string(),
                        clean_both,
                    ),
                }
            }
            .await;
            let Some(watcher) = watcher else {
                return; // 无订阅者: 操作已完, 事件无人收
            };
            let _ = messaging.tell(Envelope::new(
                Route::Instance(watcher),
                Address::instance(instance),
                KIND_GIT_OP_FINISHED,
                Payload::new(GitOpFinished {
                    action_id,
                    target,
                    ok,
                    detail,
                    statuses,
                }),
            ));
        });
        self.ctx.messaging.spawn_task(fut);
    }

    /// 后台完成事件回投自己后的落地（create/delete 专用；pull/push 直发
    /// 订阅者不经此路）：成功 → 重建快照发 `TreeChanged`（层级真变），
    /// 然后把完成事件转发 watch 订阅者（notice 链路, 无 corr）。
    async fn on_git_op_result(&mut self, env: &Envelope) {
        let Ok(done) = env.downcast::<GitOpFinished>() else {
            let _ = self.ctx.messaging.reject(env);
            return;
        };
        let stateful = done.ok
            && (done.action_id == ACTION_CREATE_WORKTREE
                || done.action_id == ACTION_DELETE_WORKTREE);
        if stateful && self.watcher.is_some() {
            let snap = self.snapshot().await;
            self.emit(snap); // 层级真变 → TreeChanged
        }
        if let Some(watcher) = self.watcher {
            let _ = self.ctx.messaging.tell(Envelope::new(
                Route::Instance(watcher),
                Address::instance(self.ctx.instance),
                KIND_GIT_OP_FINISHED,
                Payload::new(done.clone()),
            ));
        }
    }

    /// `create`：append simple item + 落盘。幂等（spec: Create idempotent）：
    /// 同 path 已存在 → 原样回执、不重复。写失败 → `Persist` 红条回执 +
    /// **内存变更保留** + 树变更照发（workspace-management 契约）。
    async fn on_create(&mut self, env: &Envelope, invoke: &ActionInvoke) {
        // 参数在 `args` 里 — 视图构造 typed args (design D6, 与 2.5 同形);
        // 缺失或类型不对 = 契约 bug → reject (`TypeMismatch` 同类)。
        let Some(create_req) = invoke
            .args
            .as_ref()
            .and_then(|a| a.downcast::<CreateReq>())
        else {
            let _ = self.ctx.messaging.reject(env);
            return;
        };
        if valid_checker::create_is_noop(&self.items, &create_req.path) {
            // 幂等 no-op (spec: Re-adding an existing solution changes
            // nothing): 不写盘、不发树变更、原样回执
            self.reply_to(env, Ok(self.snapshot().await));
            return;
        }
        // 未绑定工作区 (load 从未成功) → 无处落盘; **先查再改内存**,
        // 不留"内存已改但没落盘"的半吊子状态 (design D5)。
        let Some(ws_dir) = self.workspace_dir.clone() else {
            self.reply_to(env, Err(OpError::Persist(NO_BOUND_WORKSPACE.into())));
            return;
        };
        self.items.push(SolutionItem::Simple {
            path: create_req.path.clone(),
            register_to_asset_manager: true,
            git_enabled: true,
        });
        let snap = self.snapshot().await;
        match loader::save_items(&ws_dir, &self.items) {
            Ok(()) => {
                self.reply_to(env, Ok(snap.clone()));
                self.emit(snap);
            }
            Err(e) => {
                self.reply_to(env, Err(OpError::Persist(e)));
                self.emit(snap);
            }
        }
    }

    /// `configure`：改配置项（None = 不改）。同值重放 = no-op（spec:
    /// Configure idempotent —— 无写盘、无树变更、不 re-trigger handoff）。
    /// 写失败语义同 create（内存保留 + 红条）。
    async fn on_configure(&mut self, env: &Envelope, invoke: &ActionInvoke) {
        // 目标单源 = `node` (design D4): 格式坏 = 契约 bug → reject;
        // 前缀对但无此 solution → 用户可见红条 `NoSuch`。
        let Some(target) = parse_solution_node(invoke.node.as_deref()) else {
            let _ = self.ctx.messaging.reject(env);
            return;
        };
        if !self.items.iter().any(|i| i.path() == target.as_path()) {
            self.reply_to(env, Err(OpError::NoSuch(target)));
            return;
        }
        let Some(cfg) = invoke.args.as_ref().and_then(|a| a.downcast::<ConfigureReq>()) else {
            let _ = self.ctx.messaging.reject(env);
            return;
        };
        // `action_id` 选改哪个 flag, 值取自 `args` — 视图在点击时由
        // 描述符的 `checked` 取反构造 (design D4 修订: 与 2.5 同形)。
        let (register, git) = match invoke.action_id.as_str() {
            ACTION_CONFIGURE_REGISTER => (cfg.register_to_asset_manager, None),
            ACTION_CONFIGURE_GIT => (None, cfg.git_enabled),
            other => {
                self.reply_to(env, Err(OpError::UnknownAction(other.to_string())));
                return;
            }
        };
        if valid_checker::configure_is_noop(&self.items, &target, register, git) {
            // 幂等 no-op: 状态/文件/交树均不变 (spec), 不发树变更
            self.reply_to(env, Ok(self.snapshot().await));
            return;
        }
        // 未绑定 → 无处落盘; **先查再改内存** (同 on_create)。
        let Some(ws_dir) = self.workspace_dir.clone() else {
            self.reply_to(env, Err(OpError::Persist(NO_BOUND_WORKSPACE.into())));
            return;
        };
        for item in &mut self.items {
            if item.path() != target.as_path() {
                continue;
            }
            if let Some(v) = register {
                item.set_register_to_asset_manager(v);
            }
            if let Some(v) = git {
                item.set_git_enabled(v);
            }
        }
        let snap = self.snapshot().await;
        match loader::save_items(&ws_dir, &self.items) {
            Ok(()) => {
                self.reply_to(env, Ok(snap.clone()));
                self.emit(snap);
            }
            Err(e) => {
                self.reply_to(env, Err(OpError::Persist(e)));
                self.emit(snap);
            }
        }
    }

    /// `watch`：树变更订阅握手（订阅身份取信封 `from`）；首次订阅即推
    /// 全量快照（GUI 镜像冷启动零回程，对齐 `AgentService::on_watch`）。
    async fn on_watch(&mut self, env: &Envelope) {
        let Ok(_) = env.downcast::<WatchReq>() else {
            let _ = self.ctx.messaging.reject(env);
            return;
        };
        match *env.from.route() {
            Route::Instance(id) => {
                self.watcher = Some(id);
                self.emit(self.snapshot().await);
            }
            _ => {
                let _ = self.ctx.messaging.reject(env);
            }
        }
    }
}

impl Service for SolutionService {
    fn start(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn handle(&mut self, env: Envelope) -> ServiceFuture<'_> {
        Box::pin(self.dispatch(env))
    }
}

/// 工厂（对齐 `agent_service_factory`；`ServiceOrigin::Internal` 编译期函数指针）。
pub fn solution_service_factory(ctx: ServiceCtx) -> Box<dyn Service> {
    Box::new(SolutionService::new(ctx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{ACTION_CREATE_WORKTREE, ACTION_PULL, ACTION_PUSH, LoadError,
        solution_node_id};
    use crate::testkit::{ask_env, test_ctx, TestMessaging};
    use std::path::PathBuf;

    /// 每测试独立 temp 根（进程 id + 名字，平行安全）。
    fn temp_ws(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sol-svc-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 造一个真实项目目录（simple item 的 path 必须存在，spec）。
    fn make_project(ws: &std::path::Path, name: &str) -> PathBuf {
        let root = ws.join(name);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    /// 驱动一条信封至完成：`handle` 返回真实 future（snapshot 要 await
    /// 阻塞池里的 worktree 列表），经 testkit 的 runtime 跑完（change
    /// solution-git-offload D1）。
    fn drive(svc: &mut SolutionService, env: service_core::Envelope) {
        crate::testkit::block_on(svc.handle(env));
    }

    /// 取最后一条 KIND_OP_REPLY 回执（多轮 ask 后取最新轮）。
    fn last_reply(msg: &TestMessaging) -> OpResult {
        msg.sent()
            .into_iter()
            .rfind(|e| e.kind == KIND_OP_REPLY)
            .expect("应有回执")
            .downcast::<OpResult>()
            .expect("回执类型")
            .clone()
    }

    /// 写入口单点后的 create 信封（design D6）：`KIND_ACTION` + `args`，
    /// `node = None`（workspace 级动作, design D4）。
    fn create_env(path: PathBuf) -> service_core::Envelope {
        ask_env(
            KIND_ACTION,
            ActionInvoke {
                node: None,
                action_id: ACTION_CREATE.to_string(),
                args: Some(Payload::new(CreateReq { path })),
            },
        )
    }

    /// configure 动作信封：目标在 `node`（design D4 目标单源），值在 `args`。
    ///
    /// `action_id` 选改哪个 flag — 与 UI 的单个菜单项一一对应, 一次 invoke
    /// 只动一个 flag。
    fn configure_env(
        target: &std::path::Path,
        action_id: &str,
        cfg: ConfigureReq,
    ) -> service_core::Envelope {
        ask_env(
            KIND_ACTION,
            ActionInvoke {
                node: Some(solution_node_id(target)),
                action_id: action_id.to_string(),
                args: Some(Payload::new(cfg)),
            },
        )
    }

    /// task 2.1 验证：Load/GetTree 回执（成功态）。
    #[test]
    fn load_and_get_tree_reply_snapshot() {
        let ws = temp_ws("load");
        let foo = make_project(&ws, "foo");
        std::fs::write(
            loader::solutions_path(&ws),
            serde_json::json!([{"type": "simple", "path": foo}]).to_string(),
        )
        .unwrap();
        let (ctx, msg) = test_ctx(1);
        let mut svc = SolutionService::new(ctx);

        drive(&mut svc, ask_env(KIND_LOAD, LoadReq { workspace_dir: ws.clone() }));
        let Ok(snap) = last_reply(&msg) else {
            panic!("load 应成功");
        };
        assert_eq!(snap.tree().projects.len(), 1);
        assert_eq!(snap.tree().projects[0].name, "foo");
        assert_eq!(snap.handoff().projects.len(), 1);

        drive(&mut svc, ask_env(KIND_GET_TREE, GetTreeReq));
        assert_eq!(last_reply(&msg), Ok(snap), "GetTree 与 Load 回执同源同值");
    }

    /// 2.2: 服务 `snapshot()` 的节点携带描述符与 label — 右键菜单零往返
    /// 的数据源（design D3; 走真实构造路径, 不是契约侧的 helper）。
    #[test]
    fn snapshot_carries_action_descriptors_on_solution_nodes() {
        use crate::contract::{ACTION_CONFIGURE_GIT, ACTION_CONFIGURE_REGISTER};

        let ws = temp_ws("actions");
        let foo = make_project(&ws, "foo");
        // 缺 flag 默认 enabled (spec: legacy item without flags)
        std::fs::write(
            loader::solutions_path(&ws),
            serde_json::json!([{"type": "simple", "path": foo}]).to_string(),
        )
        .unwrap();
        let (ctx, msg) = test_ctx(2);
        let mut svc = SolutionService::new(ctx);

        drive(&mut svc, ask_env(KIND_LOAD, LoadReq { workspace_dir: ws.clone() }));
        let Ok(snap) = last_reply(&msg) else {
            panic!("load 应成功");
        };

        let root = &snap.hierarchy[0];
        let ids: Vec<&str> = root.actions.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![ACTION_CONFIGURE_REGISTER, ACTION_CONFIGURE_GIT],
            "solution 节点声明两个动作"
        );
        assert_eq!(root.actions[0].label, "注册到资产树", "描述符携带 label");
        assert_eq!(
            root.actions[0].checked,
            Some(true),
            "缺省 flag = enabled → checked 为 true"
        );
        assert_eq!(
            root.projects[0]
                .actions
                .iter()
                .map(|a| a.id.as_str())
                .collect::<Vec<_>>(),
            vec![ACTION_PULL, ACTION_PUSH, ACTION_CREATE_WORKTREE],
            "project 节点恒声明三个 git 动作 (change solution-git-worktree 2.1)"
        );
    }

    /// task 2.1 验证：失败回执只有结构化 reason，不缓存部分树（spec:
    /// Solution load failures）。
    #[test]
    fn load_failure_replies_reason_without_partial_tree() {
        let ws = temp_ws("fail");
        std::fs::write(
            loader::solutions_path(&ws),
            r#"[{"type": "simple", "path": "/gone-never-existed"}]"#,
        )
        .unwrap();
        let (ctx, msg) = test_ctx(2);
        let mut svc = SolutionService::new(ctx);

        drive(&mut svc, ask_env(KIND_LOAD, LoadReq { workspace_dir: ws.clone() }));
        assert!(
            matches!(last_reply(&msg), Err(OpError::Load(LoadError::MissingPath(_)))),
            "死 path → MissingPath 结构化 reason"
        );

        // 失败后状态未被部分覆盖（spec: no partial project tree cached）
        drive(&mut svc, ask_env(KIND_GET_TREE, GetTreeReq));
        assert_eq!(
            last_reply(&msg),
            Ok(TreeSnapshot::default()),
            "失败的 load 不留部分树"
        );
    }

    /// 2.1: load **绑定**工作区目录 (design D5) —
    /// 失败不记目录；绑定后 create/configure 落到该目录。
    #[test]
    fn load_binds_workspace_dir_only_on_success() {
        let ws = temp_ws("bind");
        let foo = make_project(&ws, "foo");
        let (ctx, msg) = test_ctx(7);
        let mut svc = SolutionService::new(ctx);

        // (a) load 失败 → 目录**未**记录
        std::fs::write(loader::solutions_path(&ws), "{oops").unwrap();
        drive(&mut svc, ask_env(KIND_LOAD, LoadReq { workspace_dir: ws.clone() }));
        assert!(
            matches!(last_reply(&msg), Err(OpError::Load(_))),
            "坏 solutions → load 失败"
        );

        // 未绑定 → create 无处落盘; 且**不改内存** (先查再改)
        drive(&mut svc, create_env(foo.clone()));
        assert!(
            matches!(last_reply(&msg), Err(OpError::Persist(_))),
            "未绑定不得 create"
        );
        drive(&mut svc, ask_env(KIND_GET_TREE, GetTreeReq));
        assert_eq!(
            last_reply(&msg).unwrap().tree().projects.len(),
            0,
            "失败不得留半改状态"
        );

        // (b) load 成功 → 绑定 → create 落进**绑定的**目录
        std::fs::write(loader::solutions_path(&ws), "[]").unwrap();
        drive(&mut svc, ask_env(KIND_LOAD, LoadReq { workspace_dir: ws.clone() }));
        assert!(last_reply(&msg).is_ok(), "合法空 solutions 应 load 成功");

        drive(&mut svc, create_env(foo.clone()));
        assert!(last_reply(&msg).is_ok(), "已绑定 → create 成功");

        let on_disk = loader::load_items(&ws).expect("落盘可读");
        assert_eq!(on_disk.len(), 1, "create 应写进绑定的目录");
        assert_eq!(on_disk[0].path(), foo.as_path());
    }

    /// 2.3 + 2.4: 三层错误归属 (design D4/D6, 实施期定案 C) —
    /// 未知 action id **即时回执**; node 格式坏 **reject 落死信**;
    /// node 对但无此 solution 回 `NoSuch`。三者都不改状态。
    #[test]
    fn action_error_attribution_is_three_way() {
        let ws = temp_ws("act-err");
        let foo = make_project(&ws, "foo");
        std::fs::write(
            loader::solutions_path(&ws),
            serde_json::json!([{"type": "simple", "path": foo}]).to_string(),
        )
        .unwrap();
        let (ctx, msg) = test_ctx(6);
        let mut svc = SolutionService::new(ctx);
        drive(&mut svc, ask_env(KIND_LOAD, LoadReq { workspace_dir: ws.clone() }));
        assert!(last_reply(&msg).is_ok(), "load 应成功");

        let reply_count = |msg: &TestMessaging| {
            msg.sent().iter().filter(|e| e.kind == KIND_OP_REPLY).count()
        };

        // (a) 服务未声明的 action id → **立即**回执 UnknownAction (不等 5s 超时)
        drive(&mut svc, ask_env(
            KIND_ACTION,
            ActionInvoke {
                node: Some(solution_node_id(&foo)),
                action_id: "does.not.exist".to_string(),
                args: None,
            },
        ));
        assert!(
            matches!(last_reply(&msg), Err(OpError::UnknownAction(a)) if a == "does.not.exist"),
            "未知 action id 即时回执并点名, 实际: {:?}",
            last_reply(&msg)
        );

        // (b) node 格式坏 → reject (不产生回执), 但**必须可观测** —
        // spec: the failure is reported rather than silently ignored
        let before = reply_count(&msg);
        let asks_before = msg
            .sent()
            .iter()
            .filter(|e| e.kind == KIND_ACTION)
            .count();
        drive(&mut svc, ask_env(
            KIND_ACTION,
            ActionInvoke {
                node: Some("project://not-a-solution".to_string()),
                action_id: ACTION_CONFIGURE_REGISTER.to_string(),
                args: None,
            },
        ));
        assert_eq!(
            reply_count(&msg),
            before,
            "node 格式坏 → reject, 不回信封 (与 downcast 失败同语义)"
        );
        assert_eq!(
            msg.sent().iter().filter(|e| e.kind == KIND_ACTION).count(),
            asks_before + 1,
            "reject 仍入出站队列 (可观测) — 不是静默丢弃"
        );

        // (c) node 前缀对但无此 solution → 用户可见 NoSuch
        drive(&mut svc, ask_env(
            KIND_ACTION,
            ActionInvoke {
                node: Some(solution_node_id(std::path::Path::new("/a/gone"))),
                action_id: ACTION_CONFIGURE_REGISTER.to_string(),
                args: None,
            },
        ));
        assert!(
            matches!(last_reply(&msg), Err(OpError::NoSuch(_))),
            "无此 solution → NoSuch, 实际: {:?}",
            last_reply(&msg)
        );

        // 三层都不得改状态
        drive(&mut svc, ask_env(KIND_GET_TREE, GetTreeReq));
        assert_eq!(
            last_reply(&msg).unwrap().tree().projects.len(),
            1,
            "三层错误都不改状态"
        );
    }

    /// task 2.1 验证：状态单份——items 是唯一真值，load 整体替换、
    /// create 增量、GetTree 同源（design D7）。
    #[test]
    fn state_is_single_source_of_truth() {
        let ws = temp_ws("state");
        let foo = make_project(&ws, "foo");
        let bar = make_project(&ws, "bar");
        std::fs::write(
            loader::solutions_path(&ws),
            serde_json::json!([{"type": "simple", "path": foo}]).to_string(),
        )
        .unwrap();
        let (ctx, msg) = test_ctx(3);
        let mut svc = SolutionService::new(ctx);

        drive(&mut svc, ask_env(KIND_LOAD, LoadReq { workspace_dir: ws.clone() }));
        assert_eq!(last_reply(&msg).unwrap().tree().projects.len(), 1);

        // create 增量
        drive(&mut svc, create_env(bar.clone()));
        assert_eq!(last_reply(&msg).unwrap().tree().projects.len(), 2);

        // load 整体替换（新工作区只有 1 项）
        let ws2 = temp_ws("state2");
        let qux = make_project(&ws2, "qux");
        std::fs::write(
            loader::solutions_path(&ws2),
            serde_json::json!([{"type": "simple", "path": qux}]).to_string(),
        )
        .unwrap();
        drive(&mut svc, ask_env(KIND_LOAD, LoadReq { workspace_dir: ws2 }));
        let snap = last_reply(&msg).unwrap();
        assert_eq!(snap.tree().projects.len(), 1, "load 替换而非追加");
        assert_eq!(snap.tree().projects[0].name, "qux");
    }

    /// task 3.2 验证：重复 Create 不产生重复 solution/project（spec:
    /// Re-adding an existing solution changes nothing——无重复交树）。
    #[test]
    fn duplicate_create_is_noop() {
        use crate::contract::{WatchReq, KIND_TREE_CHANGED, KIND_WATCH};

        let ws = temp_ws("dup-create");
        let foo = make_project(&ws, "foo");
        let (ctx, msg) = test_ctx(4);
        let mut svc = SolutionService::new(ctx);

        // watch 握手 (订阅身份 = 99) — 让树变更事件可观测
        drive(&mut svc, ask_env(KIND_WATCH, WatchReq));

        // 绑定工作区 (design D5): create 的落盘目标来自绑定, 不再随请求带
        drive(&mut svc, ask_env(KIND_LOAD, LoadReq { workspace_dir: ws.clone() }));

        drive(&mut svc, create_env(foo.clone()));
        let first = last_reply(&msg).unwrap();
        assert_eq!(first.tree().projects.len(), 1);
        let tells_after_first = msg
            .sent()
            .iter()
            .filter(|e| e.kind == KIND_TREE_CHANGED)
            .count();
        assert!(tells_after_first >= 1, "首次 create 应发树变更");

        // 重复 create → 幂等: 不重复、不写盘、不再发树变更
        drive(&mut svc, create_env(foo.clone()));
        let second = last_reply(&msg).unwrap();
        assert_eq!(second.tree().projects.len(), 1, "不产生重复 project");
        assert_eq!(second, first, "回执同值 (原样返回)");
        let text = std::fs::read_to_string(loader::solutions_path(&ws)).unwrap();
        assert_eq!(text.matches("\"path\"").count(), 1, "solutions 文件不重复");
        let tells_after_second = msg
            .sent()
            .iter()
            .filter(|e| e.kind == KIND_TREE_CHANGED)
            .count();
        assert_eq!(
            tells_after_second, tells_after_first,
            "重复 create 不发树变更 (不重复交树, spec)"
        );
    }

    /// task 3.2 验证：重复 Configure 无状态变化且不 re-trigger handoff
    /// (spec: Re-applying the same configuration is a no-op)。
    #[test]
    fn duplicate_configure_is_noop() {
        use crate::contract::{ConfigureReq, WatchReq, KIND_TREE_CHANGED, KIND_WATCH};

        let ws = temp_ws("dup-config");
        let foo = make_project(&ws, "foo");
        let (ctx, msg) = test_ctx(5);
        let mut svc = SolutionService::new(ctx);
        drive(&mut svc, ask_env(KIND_WATCH, WatchReq));
        drive(&mut svc, ask_env(
            KIND_LOAD,
            LoadReq {
                workspace_dir: ws.clone(),
            },
        ));
        // 先写入合法 solutions 文件并重载 (load 需真实文件)
        std::fs::write(
            loader::solutions_path(&ws),
            serde_json::json!([{"type": "simple", "path": foo}]).to_string(),
        )
        .unwrap();
        drive(&mut svc, ask_env(
            KIND_LOAD,
            LoadReq {
                workspace_dir: ws.clone(),
            },
        ));

        // 真实变更: 关注册 → 发树变更
        // 一次 invoke 只动一个 flag (X 定案): action_id 选 flag, 另一字段留 None
        let cfg = ConfigureReq {
            register_to_asset_manager: Some(false),
            git_enabled: None,
        };
        drive(&mut svc, configure_env(&foo.clone(), ACTION_CONFIGURE_REGISTER, cfg));
        let after_change = last_reply(&msg).unwrap();
        assert!(after_change.handoff().projects.is_empty(), "关注册后交树收缩");
        let tells_after_change = msg
            .sent()
            .iter()
            .filter(|e| e.kind == KIND_TREE_CHANGED)
            .count();

        // 同值重放 → no-op: 状态不变、不再发树变更
        let cfg2 = ConfigureReq {
            register_to_asset_manager: Some(false),
            git_enabled: None,
        };
        drive(&mut svc, configure_env(&foo.clone(), ACTION_CONFIGURE_REGISTER, cfg2));
        let after_replay = last_reply(&msg).unwrap();
        assert_eq!(after_replay, after_change, "状态无变化");
        let tells_after_replay = msg
            .sent()
            .iter()
            .filter(|e| e.kind == KIND_TREE_CHANGED)
            .count();
        assert_eq!(
            tells_after_replay, tells_after_change,
            "同值重放不发树变更 (不 re-trigger handoff, spec)"
        );
    }

    /// task 4.3 验证: 注册门控两 scenario (spec: Asset registration flag
    /// gates project handoff)。
    #[test]
    fn registration_gate_scenarios() {
        let ws = temp_ws("gate");
        let foo = make_project(&ws, "foo");
        let bar = make_project(&ws, "bar");
        // foo 关注册, bar 开 — 缺省双 true 的反面对照
        std::fs::write(
            loader::solutions_path(&ws),
            serde_json::json!([
                {"type": "simple", "path": foo, "register_to_asset_manager": false},
                {"type": "simple", "path": bar}
            ])
            .to_string(),
        )
        .unwrap();
        let (ctx, msg) = test_ctx(6);
        let mut svc = SolutionService::new(ctx);

        // scenario 1: Disabled solution stays out of the asset tree
        drive(&mut svc, ask_env(KIND_LOAD, LoadReq { workspace_dir: ws.clone() }));
        let snap = last_reply(&msg).unwrap();
        assert_eq!(snap.tree().projects.len(), 2, "全量集含双项目 (solution 自己的 project 集)");
        assert_eq!(snap.handoff().projects.len(), 1, "交树只含注册开启的");
        assert_eq!(snap.handoff().projects[0].name, "bar");

        // scenario 2: Enabling mid-session registers on next handoff (不重启)
        drive(&mut svc, configure_env(
            &foo.clone(),
            ACTION_CONFIGURE_REGISTER,
            ConfigureReq {
                register_to_asset_manager: Some(true),
                git_enabled: None,
            },
        ));
        let snap = last_reply(&msg).unwrap();
        assert_eq!(snap.handoff().projects.len(), 2, "enable 后下次交树出现 (不重启)");
    }

    /// 测试 setup 同步驱动 git 出口：异步出口 + testkit executor（同步
    /// git 已模块私有，change solution-git-offload D1 的编译期墙）。
    fn git_ok(args: &[&str], cwd: &std::path::Path) -> Result<String, crate::git::GitError> {
        crate::testkit::block_on(crate::git::run_ok_async(args, cwd))
    }

    /// 造一个真实 git 仓项目（simple item path 必须存在且是 git 目录）。
    fn make_git_project(ws: &std::path::Path, name: &str) -> PathBuf {
        let root = make_project(ws, name);
        let g = |args: &[&str]| {
            git_ok(args, &root).unwrap_or_else(|e| panic!("git {args:?}: {e}"))
        };
        g(&["init", "-q"]);
        g(&["config", "user.email", "t@t"]);
        g(&["config", "user.name", "t"]);
        std::fs::write(root.join("f.txt"), "v1").unwrap();
        g(&["add", "f.txt"]);
        g(&["commit", "-q", "-m", "init"]);
        root
    }

    /// pull/push 动作信封：目标在 `node`（project://<root>, 目标单源）。
    fn git_op_env(root: &std::path::Path, action_id: &str) -> service_core::Envelope {
        ask_env(
            KIND_ACTION,
            ActionInvoke {
                node: Some(crate::contract::project_node_id(root)),
                action_id: action_id.to_string(),
                args: None,
            },
        )
    }

    /// change solution-git-worktree 3.1 + solution-git-offload D2 验证：
    /// 门控三态 —— 干净仓接受 / tracked 脏**接受后由后台门控拒**（完成
    /// 事件 notice, 仓库零改动）/ 非 git 项目拒（同步, 秒判）。
    #[test]
    fn pull_push_gate_three_states() {
        use crate::contract::{KIND_GIT_OP_FINISHED, WatchReq, KIND_WATCH};

        let ws = temp_ws("pull-gate"); // 不得与 registration_gate_scenarios 撞名 (同 pid 同目录, 并行互踩)
        let clean = make_git_project(&ws, "clean");
        let dirty = make_git_project(&ws, "dirty");
        let plain = make_project(&ws, "plain"); // 非 git
        std::fs::write(
            loader::solutions_path(&ws),
            serde_json::json!([
                {"type": "simple", "path": clean},
                {"type": "simple", "path": dirty},
                {"type": "simple", "path": plain}
            ])
            .to_string(),
        )
        .unwrap();
        let (ctx, msg) = test_ctx(7);
        let mut svc = SolutionService::new(ctx);
        // 订阅完成事件 (门控拒绝 = notice, 只发订阅者)
        drive(&mut svc, ask_env(KIND_WATCH, WatchReq));
        drive(&mut svc, ask_env(KIND_LOAD, LoadReq { workspace_dir: ws.clone() }));
        assert!(last_reply(&msg).is_ok(), "load 应成功");

        // 态1: 干净 git 仓 → 接受 (Ok 回执; 门控 + 执行在后台)
        drive(&mut svc, git_op_env(&clean, ACTION_PULL));
        assert!(
            last_reply(&msg).is_ok(),
            "干净仓 pull 应被接受: {:?}",
            last_reply(&msg)
        );

        // 态2: tracked 脏 → **秒回 accepted**, 后台门控拒 → 完成事件
        // notice (执行好后再反馈), 且仓库零改动 (门控不碰仓)
        std::fs::write(dirty.join("f.txt"), "v2-uncommitted").unwrap();
        let status_before = git_ok(&["status", "--porcelain"], &dirty).unwrap();
        drive(&mut svc, git_op_env(&dirty, ACTION_PUSH));
        assert!(
            last_reply(&msg).is_ok(),
            "门控后台化 → accepted 先回: {:?}",
            last_reply(&msg)
        );
        msg.drive_spawned(); // 门控 + 执行跑完
        let refusal = msg
            .sent()
            .into_iter()
            .filter(|e| e.kind == KIND_GIT_OP_FINISHED)
            .filter_map(|e| e.downcast::<crate::contract::GitOpFinished>().ok().cloned())
            .find(|d| d.action_id == ACTION_PUSH && d.target == dirty.display().to_string())
            .expect("脏仓应有完成事件");
        assert!(!refusal.ok, "门控拒 → ok=false");
        assert!(
            refusal.detail.contains("未提交"),
            "拒绝文案点名 tracked 改动: {}",
            refusal.detail
        );
        assert_eq!(refusal.statuses.len(), 1, "门判出的仓 → 单条状态");
        assert_eq!(refusal.statuses[0].path, dirty.display().to_string());
        assert_eq!(
            refusal.statuses[0].state,
            RepoState::Dirty,
            "门禁判出的仓 → Dirty 徽标"
        );
        let status_after = git_ok(&["status", "--porcelain"], &dirty).unwrap();
        assert_eq!(status_before, status_after, "拒时仓库必须零改动");

        // 态3: 非 git 项目 → 拒 (同步秒判, 不 spawn)
        drive(&mut svc, git_op_env(&plain, ACTION_PULL));
        match last_reply(&msg) {
            Err(OpError::Git(m)) => assert!(m.contains("不是 git"), "红条点名非 git: {m}"),
            other => panic!("非 git 项目应拒: {other:?}"),
        }

        // 附: node 格式坏 → reject (契约 bug, 与既有归因一致)
        drive(&mut svc, ask_env(
            KIND_ACTION,
            ActionInvoke {
                node: Some("solution://looks-like-solution".into()),
                action_id: ACTION_PULL.to_string(),
                args: None,
            },
        ));
        // reject 不回信封 —— 只要没新增回执即为 reject 路径
        let replies = msg
            .sent()
            .iter()
            .filter(|e| e.kind == KIND_OP_REPLY)
            .count();
        assert_eq!(
            replies, 4,
            "四次回执 (load + 两接受 + 一拒), 第五发应 reject 无回执"
        );
    }

    /// change solution-git-worktree 3.2 验证：接受回执先于完成事件、
    /// 完成事件无 corr（design D1）、失败事件携带 git stderr 摘要。
    #[test]
    fn git_op_replies_accepted_then_finishes_out_of_band() {
        use crate::contract::{GitOpFinished, KIND_GIT_OP_FINISHED, WatchReq, KIND_WATCH};

        let ws = temp_ws("async-op");
        let repo = make_git_project(&ws, "repo");
        std::fs::write(
            loader::solutions_path(&ws),
            serde_json::json!([{"type": "simple", "path": repo}]).to_string(),
        )
        .unwrap();
        let (ctx, msg) = test_ctx(8);
        let mut svc = SolutionService::new(ctx);

        // watch 握手 —— 完成事件只发订阅者 (同 emit 立场)
        drive(&mut svc, ask_env(KIND_WATCH, WatchReq));
        drive(&mut svc, ask_env(KIND_LOAD, LoadReq { workspace_dir: ws.clone() }));
        assert!(last_reply(&msg).is_ok());
        let replies_before = msg
            .sent()
            .iter()
            .filter(|e| e.kind == KIND_OP_REPLY)
            .count();

        // 无 remote 的仓 push 必失败 (无网络依赖) —— 走失败完成事件
        drive(&mut svc, git_op_env(&repo, ACTION_PUSH));
        // ① 秒回 accepted: handle 返回即有回执, 后台任务未 drive
        let replies_now = msg
            .sent()
            .iter()
            .filter(|e| e.kind == KIND_OP_REPLY)
            .count();
        assert_eq!(
            replies_now,
            replies_before + 1,
            "接受回执必须在 spawn 后台任务 drive 之前同步发出"
        );
        assert!(last_reply(&msg).is_ok(), "双门过 → accepted");
        assert!(
            !msg.sent().iter().any(|e| e.kind == KIND_GIT_OP_FINISHED),
            "完成事件在后台任务 drive 前不得出现"
        );

        // ② drive 后台任务 → 完成事件
        msg.drive_spawned();
        let fin = msg
            .sent()
            .into_iter()
            .find(|e| e.kind == KIND_GIT_OP_FINISHED)
            .expect("应有完成事件");
        assert_eq!(fin.corr, None, "完成事件必须无 corr (design D1, 迟到无害)");
        let done = fin.downcast::<GitOpFinished>().expect("载荷类型");
        assert_eq!(done.action_id, ACTION_PUSH, "按 action_id 回到 notice 链路");
        assert_eq!(done.target, repo.display().to_string());
        assert!(!done.ok, "无 remote 的 push 应失败");
        assert!(
            !done.detail.trim().is_empty(),
            "失败必须携带 git stderr 摘要: {:?}",
            done.detail
        );
        assert_eq!(done.statuses.len(), 1, "门已判 Clean → 回传");
        assert_eq!(
            done.statuses[0].state,
            RepoState::Clean,
            "非冲突失败 → 保持门判的 Clean"
        );

        // ③ 成功事件: pull 在干净无远端仓…同样失败; 用已配 remote 的仓验成功
        //    —— 本地 file:// remote 无网络 (3.3 集成题覆盖拉平, 这里验形状)
        let ws2 = temp_ws("async-ok");
        let remote = ws2.join("remote.git");
        std::fs::create_dir_all(&remote).unwrap();
        git_ok(&["init", "-q", "--bare"], &remote).unwrap();
        let src = ws2.join("src");
        std::fs::create_dir_all(&src).unwrap();
        let g = |args: &[&str]| git_ok(args, &src).unwrap();
        g(&["init", "-q"]);
        g(&["config", "user.email", "t@t"]);
        g(&["config", "user.name", "t"]);
        std::fs::write(src.join("f.txt"), "v1").unwrap();
        g(&["add", "f.txt"]);
        g(&["commit", "-q", "-m", "init"]);
        g(&["remote", "add", "origin", &remote.to_string_lossy()]);
        g(&["push", "-q", "-u", "origin", "HEAD"]);
        std::fs::write(
            loader::solutions_path(&ws2),
            serde_json::json!([{"type": "simple", "path": src}]).to_string(),
        )
        .unwrap();
        let (ctx2, msg2) = test_ctx(80);
        let mut svc2 = SolutionService::new(ctx2);
        drive(&mut svc2, ask_env(KIND_WATCH, WatchReq));
        drive(&mut svc2, ask_env(KIND_LOAD, LoadReq { workspace_dir: ws2.clone() }));
        drive(&mut svc2, git_op_env(&src, ACTION_PULL));
        assert!(last_reply(&msg2).is_ok(), "干净已同步仓 pull 应接受");
        msg2.drive_spawned();
        let fin2 = msg2
            .sent()
            .into_iter()
            .find(|e| e.kind == KIND_GIT_OP_FINISHED)
            .expect("应有完成事件");
        assert_eq!(fin2.corr, None);
        let done2 = fin2.downcast::<GitOpFinished>().unwrap();
        assert!(done2.ok, "已同步仓 pull 成功: {}", done2.detail);
        assert_eq!(done2.detail, "", "成功态 detail 为空");
        assert_eq!(done2.statuses.len(), 1, "执行后的仓 → 回传");
        assert_eq!(done2.statuses[0].state, RepoState::Clean, "成功 → Clean");
    }

    /// change solution-git-worktree 3.3 验证：file:// 本地 remote 集成
    /// （无网络）—— behind → pull 拉平、ahead → push 推上、无 upstream
    /// → 失败完成事件。
    #[test]
    fn file_remote_pull_push_round_trip() {
        use crate::contract::{GitOpFinished, KIND_GIT_OP_FINISHED, WatchReq, KIND_WATCH};

        let ws = temp_ws("file-remote");
        let remote = ws.join("remote.git");
        std::fs::create_dir_all(&remote).unwrap();
        git_ok(&["init", "-q", "--bare"], &remote).unwrap();
        let remote_url = format!("file://{}", remote.display());

        // 主仓: c1 + push -u origin HEAD
        let repo = ws.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let g = |args: &[&str]| {
            git_ok(args, &repo).unwrap_or_else(|e| panic!("git {args:?}: {e}"))
        };
        g(&["init", "-q"]);
        g(&["config", "user.email", "t@t"]);
        g(&["config", "user.name", "t"]);
        std::fs::write(repo.join("f.txt"), "c1").unwrap();
        g(&["add", "f.txt"]);
        g(&["commit", "-q", "-m", "c1"]);
        g(&["remote", "add", "origin", remote_url.as_str()]);
        g(&["push", "-q", "-u", "origin", "HEAD"]);
        let branch = g(&["rev-parse", "--abbrev-ref", "HEAD"]).trim().to_string();
        let c1 = g(&["rev-parse", "HEAD"]).trim().to_string();

        // peer 仓: 推 c2 → 主仓落后 (behind)
        let peer = ws.join("peer");
        git_ok(&["clone", "-q", remote_url.as_str(), &peer.to_string_lossy()], &ws)
            .expect("clone 应成功");
        let pg = |args: &[&str]| {
            git_ok(args, &peer).unwrap_or_else(|e| panic!("peer git {args:?}: {e}"))
        };
        pg(&["config", "user.email", "t@t"]);
        pg(&["config", "user.name", "t"]);
        std::fs::write(peer.join("f.txt"), "c2").unwrap();
        pg(&["add", "f.txt"]);
        pg(&["commit", "-q", "-m", "c2"]);
        pg(&["push", "-q"]);
        let c2 = pg(&["rev-parse", "HEAD"]).trim().to_string();
        assert_ne!(c1, c2);

        std::fs::write(
            loader::solutions_path(&ws),
            serde_json::json!([{"type": "simple", "path": repo}]).to_string(),
        )
        .unwrap();
        let (ctx, msg) = test_ctx(9);
        let mut svc = SolutionService::new(ctx);
        drive(&mut svc, ask_env(KIND_WATCH, WatchReq));
        drive(&mut svc, ask_env(KIND_LOAD, LoadReq { workspace_dir: ws.clone() }));
        assert!(last_reply(&msg).is_ok());

        /// 取最新一条完成事件。
        fn last_fin(msg: &TestMessaging) -> GitOpFinished {
            msg.sent()
                .into_iter()
                .rev()
                .find(|e| e.kind == KIND_GIT_OP_FINISHED)
                .expect("应有完成事件")
                .downcast::<GitOpFinished>()
                .expect("载荷类型")
                .clone()
        }

        // phase A: behind → pull 拉平
        drive(&mut svc, git_op_env(&repo, ACTION_PULL));
        assert!(last_reply(&msg).is_ok(), "落后仓 pull 应接受");
        msg.drive_spawned();
        let a = last_fin(&msg);
        assert!(a.ok, "pull 应成功: {}", a.detail);
        let head = g(&["rev-parse", "HEAD"]).trim().to_string();
        assert_eq!(head, c2, "pull 后主仓应拉平到远端 tip");

        // phase B: ahead → push 推上
        std::fs::write(repo.join("f.txt"), "c3").unwrap();
        g(&["add", "f.txt"]);
        g(&["commit", "-q", "-m", "c3"]);
        let c3 = g(&["rev-parse", "HEAD"]).trim().to_string();
        drive(&mut svc, git_op_env(&repo, ACTION_PUSH));
        assert!(last_reply(&msg).is_ok(), "领先仓 push 应接受");
        msg.drive_spawned();
        let b = last_fin(&msg);
        assert!(b.ok, "push 应成功: {}", b.detail);
        let remote_tip = git_ok(
            &["-C", &remote.to_string_lossy(), "rev-parse", &format!("refs/heads/{branch}")],
            &ws,
        )
        .unwrap()
        .trim()
        .to_string();
        assert_eq!(remote_tip, c3, "push 后远端应到新 tip");

        // phase C: 无 upstream → 失败完成事件 (带 git 原话)
        g(&["checkout", "-q", "-b", "no-upstream"]);
        drive(&mut svc, git_op_env(&repo, ACTION_PUSH));
        assert!(last_reply(&msg).is_ok(), "门只管 is_git+clean, 接受");
        msg.drive_spawned();
        let c = last_fin(&msg);
        assert!(!c.ok, "无 upstream push 应失败");
        assert_eq!(c.action_id, ACTION_PUSH);
        assert!(
            !c.detail.trim().is_empty(),
            "失败必须携带 git stderr: {:?}",
            c.detail
        );
    }

    /// worktree 动作信封：create 目标 = `project://<root>` + typed args；
    /// delete 目标 = worktree 裸路径（solution-overview D2）。
    fn create_wt_env(root: &std::path::Path, name: &str) -> service_core::Envelope {
        ask_env(
            KIND_ACTION,
            ActionInvoke {
                node: Some(crate::contract::project_node_id(root)),
                action_id: crate::contract::ACTION_CREATE_WORKTREE.to_string(),
                args: Some(Payload::new(crate::contract::CreateWorktreeReq {
                    name: name.to_string(),
                })),
            },
        )
    }

    fn delete_wt_env(path: &std::path::Path) -> service_core::Envelope {
        ask_env(
            KIND_ACTION,
            ActionInvoke {
                node: Some(path.display().to_string()),
                action_id: crate::contract::ACTION_DELETE_WORKTREE.to_string(),
                args: None,
            },
        )
    }

    /// 裸路径节点 + 任意动作 id（sync 双向共用）。
    fn wt_node_env(path: &std::path::Path, action_id: &str) -> service_core::Envelope {
        ask_env(
            KIND_ACTION,
            ActionInvoke {
                node: Some(path.display().to_string()),
                action_id: action_id.to_string(),
                args: None,
            },
        )
    }

    /// 从快照里取某 project 的 worktree 层。
    fn wt_layer(snap: &TreeSnapshot, root: &std::path::Path) -> Vec<WorktreeNode> {
        snap.hierarchy
            .iter()
            .flat_map(|s| s.projects.iter())
            .find(|p| p.desc.root == root)
            .map(|p| p.worktrees.to_vec())
            .unwrap_or_else(|| panic!("快照里应有 project {}", root.display()))
    }

    /// change solution-git-sync (task 3.2): `worktree.sync_from_project`
    /// 四例 —— 干净合并成功（statuses=Clean、detail 含 merge 摘要）/
    /// 脏拒绝（statuses=Dirty、仓库零改动）/ 冲突（detail 列文件 +
    /// Conflict）/ 幽灵·非 git 路径拒绝（statuses 空）。
    #[test]
    fn worktree_sync_from_project_four_cases() {
        use crate::contract::{KIND_WATCH, WatchReq};

        let ws = temp_ws("wt-sync-from");
        let repo = make_git_project(&ws, "repo");
        std::fs::write(
            loader::solutions_path(&ws),
            serde_json::json!([{"type": "simple", "path": repo}]).to_string(),
        )
        .unwrap();
        let (ctx, msg) = test_ctx(12);
        let mut svc = SolutionService::new(ctx);
        drive(&mut svc, ask_env(KIND_WATCH, WatchReq));
        drive(
            &mut svc,
            ask_env(
                KIND_LOAD,
                LoadReq {
                    workspace_dir: ws.clone(),
                },
            ),
        );
        assert!(last_reply(&msg).is_ok(), "load 应成功");

        // worktree 登记（走 create 动作 → TreeChanged 取层）
        drive(&mut svc, create_wt_env(&repo, "feat"));
        drive_bg(&mut svc, &msg, 12);
        let snap = last_tree_changed(&msg);
        let wt_path = wt_layer(&snap, &repo)[0].path.clone();
        let sync = || wt_node_env(&wt_path, ACTION_SYNC_WORKTREE_FROM_PROJECT);

        // 两侧分叉: wt 加 g.txt, project 加 h.txt（不同文件 → 合并无冲突）
        std::fs::write(wt_path.join("g.txt"), "g").unwrap();
        git_ok(&["add", "g.txt"], &wt_path).unwrap();
        git_ok(&["commit", "-q", "-m", "wt-g"], &wt_path).unwrap();
        std::fs::write(repo.join("h.txt"), "h").unwrap();
        git_ok(&["add", "h.txt"], &repo).unwrap();
        git_ok(&["commit", "-q", "-m", "proj-h"], &repo).unwrap();

        // 场景A: 干净合并成功
        drive(&mut svc, sync());
        assert!(last_reply(&msg).is_ok(), "accepted 先回");
        drive_bg(&mut svc, &msg, 12);
        let done = find_fin(
            &msg,
            ACTION_SYNC_WORKTREE_FROM_PROJECT,
            &wt_path.display().to_string(),
        );
        assert!(done.ok, "干净合并应成功: {}", done.detail);
        assert!(
            done.detail.contains("Merge made") || done.detail.contains("Fast-forward"),
            "成功 detail 应带 merge 摘要: {}",
            done.detail
        );
        assert_eq!(done.statuses.len(), 1, "D1 只检视 worktree");
        assert_eq!(done.statuses[0].path, wt_path.display().to_string());
        assert_eq!(
            done.statuses[0].state,
            RepoState::Clean,
            "成功 → 徽标 Clean"
        );

        // 场景B: tracked 脏 → 拒, statuses=Dirty, 仓库零改动
        std::fs::write(wt_path.join("f.txt"), "dirty-uncommitted").unwrap();
        let before = git_ok(&["status", "--porcelain"], &wt_path).unwrap();
        drive(&mut svc, sync());
        assert!(last_reply(&msg).is_ok(), "门控后台化 → accepted 先回");
        drive_bg(&mut svc, &msg, 12);
        let refusal = find_fin(
            &msg,
            ACTION_SYNC_WORKTREE_FROM_PROJECT,
            &wt_path.display().to_string(),
        );
        assert!(!refusal.ok, "脏树应被拒");
        assert!(refusal.detail.contains("未提交"), "{}", refusal.detail);
        assert_eq!(refusal.statuses.len(), 1);
        assert_eq!(
            refusal.statuses[0].state,
            RepoState::Dirty,
            "拒 → 徽标 Dirty"
        );
        let after = git_ok(&["status", "--porcelain"], &wt_path).unwrap();
        assert_eq!(before, after, "拒时仓库必须零改动");
        git_ok(&["checkout", "--", "f.txt"], &wt_path).unwrap(); // 复位

        // 场景C: 双侧改 f.txt → 冲突, detail 列文件 + Conflict
        std::fs::write(wt_path.join("f.txt"), "wt-side").unwrap();
        git_ok(&["add", "f.txt"], &wt_path).unwrap();
        git_ok(&["commit", "-q", "-m", "wt-f"], &wt_path).unwrap();
        std::fs::write(repo.join("f.txt"), "proj-side").unwrap();
        git_ok(&["add", "f.txt"], &repo).unwrap();
        git_ok(&["commit", "-q", "-m", "proj-f"], &repo).unwrap();
        drive(&mut svc, sync());
        drive_bg(&mut svc, &msg, 12);
        let conflict = find_fin(
            &msg,
            ACTION_SYNC_WORKTREE_FROM_PROJECT,
            &wt_path.display().to_string(),
        );
        assert!(!conflict.ok, "冲突必须报失败");
        assert!(conflict.detail.contains("合并冲突"), "{}", conflict.detail);
        assert!(
            conflict.detail.contains("f.txt"),
            "冲突详情应列文件: {}",
            conflict.detail
        );
        assert_eq!(conflict.statuses.len(), 1);
        assert_eq!(
            conflict.statuses[0].state,
            RepoState::Conflict,
            "冲突 → 徽标 Conflict（非 Dirty）"
        );
        let mid = git_ok(&["status", "--porcelain"], &wt_path).unwrap();
        assert!(!mid.trim().is_empty(), "冲突后仓处未完成合并态");
        git_ok(&["merge", "--abort"], &wt_path).unwrap();

        // 场景D: 幽灵路径（含非 git/关了 git 管理 → 属主找不到）→ 拒
        let ghost = ws.join("ghost-wt");
        drive(
            &mut svc,
            wt_node_env(&ghost, ACTION_SYNC_WORKTREE_FROM_PROJECT),
        );
        assert!(last_reply(&msg).is_ok(), "幽灵也先 accepted");
        drive_bg(&mut svc, &msg, 12);
        let gone = find_fin(
            &msg,
            ACTION_SYNC_WORKTREE_FROM_PROJECT,
            &ghost.display().to_string(),
        );
        assert!(!gone.ok, "幽灵路径应拒");
        assert!(gone.detail.contains("no such worktree"), "{}", gone.detail);
        assert!(gone.statuses.is_empty(), "未检视任何仓 → 状态空");
    }

    /// change solution-git-sync (task 3.3): `project.sync_from_worktree`
    /// 五例 —— equal 无操作 / project 脏拒（单仓 Dirty）/ wt 脏拒
    /// （project Clean + wt Dirty）/ 落后指回 D1 / ahead 快进成功
    /// （ff + 两仓 Clean + project HEAD 跟上 wt HEAD）。
    #[test]
    fn project_sync_from_worktree_five_cases() {
        use crate::contract::{KIND_WATCH, WatchReq};

        let ws = temp_ws("proj-sync-from");
        let repo = make_git_project(&ws, "repo");
        std::fs::write(
            loader::solutions_path(&ws),
            serde_json::json!([{"type": "simple", "path": repo}]).to_string(),
        )
        .unwrap();
        let (ctx, msg) = test_ctx(13);
        let mut svc = SolutionService::new(ctx);
        drive(&mut svc, ask_env(KIND_WATCH, WatchReq));
        drive(
            &mut svc,
            ask_env(
                KIND_LOAD,
                LoadReq {
                    workspace_dir: ws.clone(),
                },
            ),
        );
        assert!(last_reply(&msg).is_ok(), "load 应成功");
        drive(&mut svc, create_wt_env(&repo, "feat"));
        drive_bg(&mut svc, &msg, 13);
        let snap = last_tree_changed(&msg);
        let wt_path = wt_layer(&snap, &repo)[0].path.clone();
        let sync = || wt_node_env(&wt_path, ACTION_SYNC_PROJECT_FROM_WORKTREE);
        let find = || {
            find_fin(
                &msg,
                ACTION_SYNC_PROJECT_FROM_WORKTREE,
                &repo.display().to_string(),
            )
        };
        let clean_both = |fin: &crate::contract::GitOpFinished, who: &str| {
            assert_eq!(fin.statuses.len(), 2, "{who}: 两仓状态都应回传");
            for want in [repo.display().to_string(), wt_path.display().to_string()] {
                let st = fin
                    .statuses
                    .iter()
                    .find(|s| s.path == want)
                    .unwrap_or_else(|| panic!("{who}: 缺 {want} 的状态"));
                assert_eq!(st.state, RepoState::Clean, "{who}: {want} 应 Clean");
            }
        };

        // 场景1: equal（两侧同 HEAD）→ 无操作拒, 两仓 Clean
        drive(&mut svc, sync());
        assert!(last_reply(&msg).is_ok(), "accepted 先回");
        drive_bg(&mut svc, &msg, 13);
        let fin = find();
        assert!(!fin.ok, "equal 应拒");
        assert!(fin.detail.contains("无可同步"), "{}", fin.detail);
        clean_both(&fin, "equal");

        // 场景2: project 脏 → 单仓 Dirty, 零改动
        std::fs::write(repo.join("f.txt"), "dirty-proj").unwrap();
        let before = git_ok(&["status", "--porcelain"], &repo).unwrap();
        drive(&mut svc, sync());
        drive_bg(&mut svc, &msg, 13);
        let fin = find();
        assert!(!fin.ok, "project 脏应拒");
        assert!(fin.detail.contains("未提交"), "{}", fin.detail);
        assert_eq!(fin.statuses.len(), 1, "门1只检视了 project");
        assert_eq!(fin.statuses[0].path, repo.display().to_string());
        assert_eq!(fin.statuses[0].state, RepoState::Dirty);
        assert_eq!(
            before,
            git_ok(&["status", "--porcelain"], &repo).unwrap(),
            "拒时仓库零改动"
        );
        git_ok(&["checkout", "--", "f.txt"], &repo).unwrap();

        // 场景3: wt 脏 → project Clean + wt Dirty
        std::fs::write(wt_path.join("f.txt"), "dirty-wt").unwrap();
        drive(&mut svc, sync());
        drive_bg(&mut svc, &msg, 13);
        let fin = find();
        assert!(!fin.ok, "wt 脏应拒");
        assert!(fin.detail.contains("未提交"), "{}", fin.detail);
        assert_eq!(fin.statuses.len(), 2, "门1过 (project Clean), 门2拒");
        let wt_st = fin
            .statuses
            .iter()
            .find(|s| s.path == wt_path.display().to_string())
            .expect("wt 状态在场");
        assert_eq!(wt_st.state, RepoState::Dirty, "wt → Dirty");
        let repo_st = fin
            .statuses
            .iter()
            .find(|s| s.path == repo.display().to_string())
            .expect("project 状态在场");
        assert_eq!(repo_st.state, RepoState::Clean, "project 门已过 → Clean");
        git_ok(&["checkout", "--", "f.txt"], &wt_path).unwrap();

        // 场景4: 落后（project 前进）→ 不含 → 指回 worktree 侧同步
        std::fs::write(repo.join("f.txt"), "proj-adv").unwrap();
        git_ok(&["add", "f.txt"], &repo).unwrap();
        git_ok(&["commit", "-q", "-m", "proj-adv"], &repo).unwrap();
        drive(&mut svc, sync());
        drive_bg(&mut svc, &msg, 13);
        let fin = find();
        assert!(!fin.ok, "落后应拒");
        assert!(
            fin.detail.contains("先在 worktree 同步 Project 代码"),
            "拒绝文案指回 D1: {}",
            fin.detail
        );
        clean_both(&fin, "behind");

        // 用 D1 把 wt 补齐（ff 到 project HEAD）, 再给 wt 一个新提交 → ahead
        drive(
            &mut svc,
            wt_node_env(&wt_path, ACTION_SYNC_WORKTREE_FROM_PROJECT),
        );
        drive_bg(&mut svc, &msg, 13);
        let d1 = find_fin(
            &msg,
            ACTION_SYNC_WORKTREE_FROM_PROJECT,
            &wt_path.display().to_string(),
        );
        assert!(d1.ok, "D1 应把落后补齐: {}", d1.detail);
        std::fs::write(wt_path.join("extra.txt"), "x").unwrap();
        git_ok(&["add", "extra.txt"], &wt_path).unwrap();
        git_ok(&["commit", "-q", "-m", "wt-extra"], &wt_path).unwrap();
        let w_head = crate::testkit::block_on(crate::git::head_sha(&wt_path)).unwrap();

        // 场景5: ahead → ff 成功, project HEAD 跟上 wt HEAD
        drive(&mut svc, sync());
        drive_bg(&mut svc, &msg, 13);
        let fin = find();
        assert!(fin.ok, "ahead 应成功: {}", fin.detail);
        assert!(fin.detail.contains("Fast-forward"), "{}", fin.detail);
        clean_both(&fin, "ahead");
        let r_head = crate::testkit::block_on(crate::git::head_sha(&repo)).unwrap();
        assert_eq!(r_head, w_head, "project 应快进到 wt 的 HEAD");
    }

    /// 跑后台任务, 并把**回投自己**的完成事件驱动回 handle
    /// (create/delete 的落地在 handle: TreeChanged + 转发订阅者 ——
    /// 模拟 driver 收信)。只驱动本次新增的信封 (mark 起点), 避免重复
    /// 落地旧事件。
    fn drive_bg(svc: &mut SolutionService, msg: &TestMessaging, self_id: u64) {
        use crate::contract::KIND_GIT_OP_FINISHED;
        let mark = msg.sent().len();
        msg.drive_spawned();
        for env in msg.sent().into_iter().skip(mark) {
            let to_self = matches!(
                env.to,
                service_core::Route::Instance(id) if id.0 == self_id
            );
            if env.kind == KIND_GIT_OP_FINISHED && to_self {
                drive(svc, env);
            }
        }
    }

    /// 最后一条 TreeChanged 的快照 (层级真变的落地载体)。
    fn last_tree_changed(msg: &TestMessaging) -> TreeSnapshot {
        msg.sent()
            .into_iter()
            .rev()
            .find(|e| e.kind == crate::contract::KIND_TREE_CHANGED)
            .expect("应有 TreeChanged")
            .downcast::<crate::contract::TreeChanged>()
            .expect("TreeChanged 载荷")
            .0
            .clone()
    }

    /// 按 action_id + target 取完成事件 (载荷克隆; watcher 转发与回投
    /// 自己的原件同载荷, 任取其一)。
    fn find_fin(msg: &TestMessaging, action_id: &str, target: &str) -> crate::contract::GitOpFinished {
        // 取**最后**一条匹配: 同 target 先拒后成时, 成功在后
        msg.sent()
            .into_iter()
            .filter(|e| e.kind == crate::contract::KIND_GIT_OP_FINISHED)
            .filter_map(|e| e.downcast::<crate::contract::GitOpFinished>().ok().cloned())
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .find(|d| d.action_id == action_id && d.target == target)
            .unwrap_or_else(|| panic!("应有完成事件 {action_id} / {target}"))
    }

    /// change solution-git-worktree 4.1 验证：create 后快照见第三层、
    /// 目录落在 `.balabala.worktrees` 下、非 git 项目拒。
    #[test]
    fn create_worktree_lands_in_workspace_and_snapshot() {
        use crate::contract::{KIND_TREE_CHANGED, WatchReq, KIND_WATCH};

        let ws = temp_ws("wt-create");
        let git_p = make_git_project(&ws, "gitproj");
        let plain = make_project(&ws, "plain");
        std::fs::write(
            loader::solutions_path(&ws),
            serde_json::json!([
                {"type": "simple", "path": git_p},
                {"type": "simple", "path": plain}
            ])
            .to_string(),
        )
        .unwrap();
        let (ctx, msg) = test_ctx(10);
        let mut svc = SolutionService::new(ctx);
        drive(&mut svc, ask_env(KIND_WATCH, WatchReq));
        drive(&mut svc, ask_env(KIND_LOAD, LoadReq { workspace_dir: ws.clone() }));
        let snap = last_reply(&msg).unwrap();
        assert!(wt_layer(&snap, &git_p).is_empty(), "初始无 worktree");
        let tells_before = msg
            .sent()
            .iter()
            .filter(|e| e.kind == KIND_TREE_CHANGED)
            .count();

        // 建 (真异步): 秒回 accepted = 操作前快照, 层级未变
        drive(&mut svc, create_wt_env(&git_p, "feature-x"));
        let snap = last_reply(&msg).expect("create 应 accepted");
        assert!(
            wt_layer(&snap, &git_p).is_empty(),
            "accepted 是操作前快照 (层级真变由后续 TreeChanged 送达)"
        );

        // 后台 add 跑完 + 回投落地 → TreeChanged 带新层级
        drive_bg(&mut svc, &msg, 10);
        let snap = last_tree_changed(&msg);
        let layer = wt_layer(&snap, &git_p);
        assert_eq!(layer.len(), 1, "TreeChanged 快照应见第三层");
        assert_eq!(layer[0].branch.as_deref(), Some("feature-x"));
        assert_eq!(layer[0].actions[0].id, crate::contract::ACTION_DELETE_WORKTREE);
        // 目录落 workspace/.balabala.worktrees 下 (design D4 点前缀)
        let dest = ws.join(crate::contract::WORKTREES_DIR).join("feature-x");
        assert!(dest.is_dir(), "目录应落 {:?}", dest);
        assert!(dest.join(".git").exists(), "应是 worktree 检出 (.git 文件)");
        // 层级真变 → TreeChanged 已发
        let tells_after = msg
            .sent()
            .iter()
            .filter(|e| e.kind == KIND_TREE_CHANGED)
            .count();
        assert!(tells_after > tells_before, "create 成功应发 TreeChanged");

        // 非 git 项目拒 (廉价校验留 handle 秒拒), 零副作用
        drive(&mut svc, create_wt_env(&plain, "nope"));
        match last_reply(&msg) {
            Err(OpError::Git(m)) => assert!(m.contains("不是 git"), "{m}"),
            other => panic!("非 git 项目应拒: {other:?}"),
        }
        assert!(
            !ws.join(crate::contract::WORKTREES_DIR).join("nope").exists(),
            "拒时不得落盘"
        );

        // 目录名消毒穿透: `feature/y` → `feature-y` (design D4)
        drive(&mut svc, create_wt_env(&git_p, "feature/y"));
        assert!(last_reply(&msg).is_ok(), "斜杠名应 accepted");
        drive_bg(&mut svc, &msg, 10);
        assert!(
            ws.join(crate::contract::WORKTREES_DIR).join("feature-y").is_dir(),
            "斜杠折叠落盘"
        );
        // 消毒后不可穿越: `..` 名直接拒 (纯 CPU 消毒留 handle 秒拒),
        // 不逃出 worktrees 目录
        drive(&mut svc, create_wt_env(&git_p, ".."));
        assert!(matches!(last_reply(&msg), Err(OpError::Git(_))), "`..` 应拒");
    }

    /// change solution-git-worktree 4.2 验证：脏树被 git 拒且目录原封、
    /// 干净树删净、快照不再列出。
    #[test]
    fn delete_worktree_gated_by_git_then_removes() {
        use crate::contract::{KIND_TREE_CHANGED, WatchReq, KIND_WATCH};

        let ws = temp_ws("wt-delete");
        let repo = make_git_project(&ws, "repo");
        std::fs::write(
            loader::solutions_path(&ws),
            serde_json::json!([{"type": "simple", "path": repo}]).to_string(),
        )
        .unwrap();
        let (ctx, msg) = test_ctx(11);
        let mut svc = SolutionService::new(ctx);
        drive(&mut svc, ask_env(KIND_WATCH, WatchReq));
        drive(&mut svc, ask_env(KIND_LOAD, LoadReq { workspace_dir: ws.clone() }));
        drive(&mut svc, create_wt_env(&repo, "doomed"));
        drive_bg(&mut svc, &msg, 11); // create 落地 → TreeChanged 带新层级
        let snap = last_tree_changed(&msg);
        let wt_path = wt_layer(&snap, &repo)[0].path.clone();
        assert!(wt_path.is_dir());
        // create 成功 → 新 worktree 状态回传（新建即净 → Clean 徽标）。
        // 按 action_id 取: create 的 target 是 parent.join 拼的 (Windows
        // 反斜杠), 快照行路径来自 git porcelain (正斜杠), 字符串不相等
        let made = msg
            .sent()
            .into_iter()
            .filter(|e| e.kind == crate::contract::KIND_GIT_OP_FINISHED)
            .filter_map(|e| e.downcast::<crate::contract::GitOpFinished>().ok().cloned())
            .find(|d| d.action_id == crate::contract::ACTION_CREATE_WORKTREE)
            .expect("应有 create 完成事件");
        assert!(made.ok, "create 应成功: {}", made.detail);
        assert_eq!(made.statuses.len(), 1, "新 worktree → 单条状态");
        assert_eq!(made.statuses[0].state, RepoState::Clean, "新建即净");

        // 脏树 → git 拒: 秒回 accepted, 后台 git 原话走完成事件 notice,
        // 目录原封, 不发 TreeChanged
        let tells_before = msg
            .sent()
            .iter()
            .filter(|e| e.kind == KIND_TREE_CHANGED)
            .count();
        std::fs::write(wt_path.join("dirty.txt"), "x").unwrap();
        drive(&mut svc, delete_wt_env(&wt_path));
        assert!(last_reply(&msg).is_ok(), "门控后台化 → accepted 先回");
        drive_bg(&mut svc, &msg, 11);
        let refusal = find_fin(
            &msg,
            crate::contract::ACTION_DELETE_WORKTREE,
            &wt_path.display().to_string(),
        );
        assert!(!refusal.ok, "脏树应被 git 拒");
        assert!(
            refusal.detail.contains("modified or untracked"),
            "git 判例原话应直达: {}",
            refusal.detail
        );
        assert!(
            refusal.statuses.is_empty(),
            "git 删除门不重推导（untracked 口径不一致）→ 不回传状态"
        );
        assert!(wt_path.is_dir(), "被拒后目录必须原封");
        let tells_mid = msg
            .sent()
            .iter()
            .filter(|e| e.kind == KIND_TREE_CHANGED)
            .count();
        assert_eq!(tells_mid, tells_before, "拒时不发 TreeChanged");

        // 干净 → 删净 + TreeChanged 快照不再列出
        std::fs::remove_file(wt_path.join("dirty.txt")).unwrap();
        drive(&mut svc, delete_wt_env(&wt_path));
        assert!(last_reply(&msg).is_ok(), "干净树应 accepted");
        drive_bg(&mut svc, &msg, 11);
        let done = find_fin(
            &msg,
            crate::contract::ACTION_DELETE_WORKTREE,
            &wt_path.display().to_string(),
        );
        assert!(done.ok, "干净树应删净: {}", done.detail);
        assert!(done.statuses.is_empty(), "仓已删 → 无状态回传");
        let snap = last_tree_changed(&msg);
        assert!(wt_layer(&snap, &repo).is_empty(), "快照不再列出");
        assert!(!wt_path.exists(), "目录删净");
        let tells_after = msg
            .sent()
            .iter()
            .filter(|e| e.kind == KIND_TREE_CHANGED)
            .count();
        assert!(tells_after > tells_mid, "删除成功应发 TreeChanged");

        // 不存在的 worktree → accepted 后台拒, 结构化原因走完成事件
        let ghost = ws.join("ghost");
        drive(&mut svc, delete_wt_env(&ghost));
        assert!(last_reply(&msg).is_ok(), "幽灵路径也先 accepted");
        drive_bg(&mut svc, &msg, 11);
        let gone = find_fin(
            &msg,
            crate::contract::ACTION_DELETE_WORKTREE,
            &ghost.display().to_string(),
        );
        assert!(!gone.ok);
        assert!(
            gone.detail.contains("no such worktree"),
            "结构化原因: {}",
            gone.detail
        );
        assert!(gone.statuses.is_empty(), "未检视任何仓 → 状态空");
    }

    /// change solution-git-offload 4.1 验证：git 子进程**不在**驱动
    /// `handle` 的线程上执行 —— 门闩卡住 `git worktree list` 入口, 分别
    /// 记录 ① 驱动线程 ② 实际执行 git 的线程, 两者必须不同（asset-
    /// service `offload_tests` 同款判据; 看门狗 3s 自动放行防挂死）。
    #[test]
    fn offload_git_subprocess_runs_off_the_driver_thread() {
        use std::time::Duration;
        // 门闩全局单例 —— 与 solution-git-sync 5.2 的出口离载测试串行
        let _probe_guard = crate::git::probe::exclusive();

        let ws = temp_ws("offload");
        let repo = make_git_project(&ws, "repo");
        std::fs::write(
            loader::solutions_path(&ws),
            serde_json::json!([{"type": "simple", "path": repo}]).to_string(),
        )
        .unwrap();
        let (ctx, msg) = test_ctx(13);
        let mut svc = SolutionService::new(ctx);
        drive(&mut svc, ask_env(KIND_LOAD, LoadReq { workspace_dir: ws.clone() }));
        assert!(last_reply(&msg).is_ok(), "load 应成功");
        let replies_before = msg
            .sent()
            .iter()
            .filter(|e| e.kind == KIND_OP_REPLY)
            .count();

        // 上闩 → 在独立线程上驱动 get_tree（该线程扮演 driver 角色）
        crate::git::probe::arm(&repo);
        let driver = std::thread::spawn(move || {
            let tid = std::thread::current().id();
            drive(&mut svc, ask_env(KIND_GET_TREE, GetTreeReq));
            tid
        });

        // 命中线程 = 实际跑 git 的线程（正确实现 = 阻塞池线程）
        let git_thread = crate::git::probe::wait_hit(Duration::from_secs(3))
            .expect("git 子进程应在门闩处命中");
        crate::git::probe::release();
        let driver_thread = driver.join().expect("drive 应完成");
        assert_ne!(
            git_thread, driver_thread,
            "git 子进程不得在驱动 handle 的线程上执行"
        );
        // 离载不改语义: get_tree 回执照常到达
        let replies_after = msg
            .sent()
            .iter()
            .filter(|e| e.kind == KIND_OP_REPLY)
            .count();
        assert_eq!(
            replies_after,
            replies_before + 1,
            "get_tree 回执应到达"
        );
    }

    /// change solution-git-worktree 4.3 验证：列表真源 = git 元数据 ——
    /// 应用**外**注册的 worktree 也出现在其 project 下。
    #[test]
    fn snapshot_lists_worktrees_registered_outside_the_app() {
        let ws = temp_ws("wt-external");
        let repo = make_git_project(&ws, "repo");
        std::fs::write(
            loader::solutions_path(&ws),
            serde_json::json!([{"type": "simple", "path": repo}]).to_string(),
        )
        .unwrap();
        // 应用外: 直接用 git 命令注册 (绕过服务)
        let ext_parent = ws.join(crate::contract::WORKTREES_DIR);
        std::fs::create_dir_all(&ext_parent).unwrap();
        crate::testkit::block_on(crate::git::worktree::add_async(
            &repo,
            &ext_parent,
            "external-one",
        ))
        .expect("应用外注册应成功");

        let (ctx, msg) = test_ctx(12);
        let mut svc = SolutionService::new(ctx);
        drive(&mut svc, ask_env(KIND_LOAD, LoadReq { workspace_dir: ws.clone() }));
        let snap = last_reply(&msg).expect("load 应成功");
        let layer = wt_layer(&snap, &repo);
        assert_eq!(layer.len(), 1, "应用外注册的 worktree 应见");
        assert_eq!(layer[0].branch.as_deref(), Some("external-one"));
        // GetTree 同源 (真源在 git, 不在服务缓存)
        drive(&mut svc, ask_env(KIND_GET_TREE, GetTreeReq));
        let snap2 = last_reply(&msg).unwrap();
        assert_eq!(wt_layer(&snap2, &repo), layer, "GetTree 与 Load 同源");
    }
}
