//! `SolutionService`：Solution/Project 域的进程内总线服务（design D1/D7/D8）。
//!
//! 状态单份归属服务端：`items` = 已加载的 solution 配置真值；项目树/交树
//! 快照由 [`Self::snapshot`] 现算（is_git 解析 + 注册门控）。WorkTree 管理
//! 后续挂接（本 change 只留口子，不建空壳）。

use std::path::PathBuf;
use std::sync::Arc;

use crate::contract::{
    ActionInvoke, ConfigureReq, CreateReq, GetTreeReq, LoadReq, OpError, OpResult, ProjectNode,
    SolutionItem, SolutionNode, TreeChanged, TreeSnapshot, WatchReq, ACTION_CONFIGURE_GIT,
    ACTION_CONFIGURE_REGISTER, ACTION_CREATE, KIND_ACTION, KIND_GET_TREE, KIND_LOAD,
    KIND_OP_REPLY, KIND_TREE_CHANGED, KIND_WATCH, parse_solution_node, solution_actions,
};
use crate::loader;
use crate::valid_checker;
use service_core::{
    Address, Envelope, Payload, Route, Service, ServiceCtx, ServiceFuture,
};

/// 未绑定工作区就发 create/configure 的回执文案（design D5）。
///
/// 正常链路（Launcher → load → 用户操作）走不到; 真到了就是调用顺序 bug —
/// 以红条暴露, 不 panic、不静默丢弃。
const NO_BOUND_WORKSPACE: &str = "no workspace bound — load the workspace first";

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
    pub fn snapshot(&self) -> TreeSnapshot {
        let hierarchy: Arc<[SolutionNode]> = self
            .items
            .iter()
            .map(|item| SolutionNode {
                item: item.clone(),
                // v1 `simple`: 1 item → 1 project (spec: Solution type
                // registry resolves items to projects)
                projects: Arc::from(vec![ProjectNode {
                    desc: loader::expand_item(item),
                    // project 节点暂无声明动作 (create worktree 属后续)
                    actions: Arc::from([]),
                }]),
                actions: solution_actions(item),
            })
            .collect::<Vec<_>>()
            .into();
        TreeSnapshot {
            solutions: self.items.clone().into(),
            hierarchy,
        }
    }

    /// 信封分发（driver 顺序上下文；未知 kind 拒收落死信，对齐 AgentService）。
    fn dispatch(&mut self, env: Envelope) {
        match env.kind {
            KIND_LOAD => self.on_load(&env),
            KIND_GET_TREE => self.on_get_tree(&env),
            KIND_ACTION => self.on_action(&env),
            KIND_WATCH => self.on_watch(&env),
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
    fn on_load(&mut self, env: &Envelope) {
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
                let snap = self.snapshot();
                self.reply_to(env, Ok(snap.clone()));
                if changed {
                    self.emit(snap);
                }
            }
        }
    }

    /// `get_tree`：镜像补拉（无状态变更）。
    fn on_get_tree(&self, env: &Envelope) {
        let Ok(_) = env.downcast::<GetTreeReq>() else {
            let _ = self.ctx.messaging.reject(env);
            return;
        };
        self.reply_to(env, Ok(self.snapshot()));
    }

    /// `solution.action` — **写入口单点**（design D6）。
    ///
    /// 分三层, 错误归属各不相同：
    /// 1. 载荷 downcast 失败 → `reject`（契约 bug, `TypeMismatch` 同类）
    /// 2. `action_id` 服务未声明 → **立即回** `OpError::UnknownAction`
    ///    （实施期定案 C — 不 reject, 见 design D6）
    /// 3. `node` 格式坏 → `reject`；前缀对但无此 solution → `NoSuch`
    ///    （那两条在 `on_create` / `on_configure` 内）
    fn on_action(&mut self, env: &Envelope) {
        let Ok(req) = env.downcast::<ActionInvoke>() else {
            let _ = self.ctx.messaging.reject(env);
            return;
        };
        match req.action_id.as_str() {
            ACTION_CREATE => self.on_create(env, &req),
            ACTION_CONFIGURE_REGISTER | ACTION_CONFIGURE_GIT => self.on_configure(env, &req),
            other => {
                self.reply_to(env, Err(OpError::UnknownAction(other.to_string())));
            }
        }
    }

    /// `create`：append simple item + 落盘。幂等（spec: Create idempotent）：
    /// 同 path 已存在 → 原样回执、不重复。写失败 → `Persist` 红条回执 +
    /// **内存变更保留** + 树变更照发（workspace-management 契约）。
    fn on_create(&mut self, env: &Envelope, invoke: &ActionInvoke) {
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
            self.reply_to(env, Ok(self.snapshot()));
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
        let snap = self.snapshot();
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
    fn on_configure(&mut self, env: &Envelope, invoke: &ActionInvoke) {
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
            self.reply_to(env, Ok(self.snapshot()));
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
        let snap = self.snapshot();
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
    fn on_watch(&mut self, env: &Envelope) {
        let Ok(_) = env.downcast::<WatchReq>() else {
            let _ = self.ctx.messaging.reject(env);
            return;
        };
        match *env.from.route() {
            Route::Instance(id) => {
                self.watcher = Some(id);
                self.emit(self.snapshot());
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
        self.dispatch(env);
        Box::pin(std::future::ready(()))
    }
}

/// 工厂（对齐 `agent_service_factory`；`ServiceOrigin::Internal` 编译期函数指针）。
pub fn solution_service_factory(ctx: ServiceCtx) -> Box<dyn Service> {
    Box::new(SolutionService::new(ctx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{LoadError, solution_node_id};
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

    /// 同步驱动一条信封 (`handle` 内部即同步 dispatch; 产出的 ready
    /// future 有意丢弃 — 测试无需 await)。
    fn drive(svc: &mut SolutionService, env: service_core::Envelope) {
        std::mem::drop(svc.handle(env));
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
            format!(r#"[{{"type": "simple", "path": "{}"}}]"#, foo.display()),
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
            format!(r#"[{{"type": "simple", "path": "{}"}}]"#, foo.display()),
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
        assert!(
            root.projects[0].actions.is_empty(),
            "project 节点暂无声明"
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
            format!(r#"[{{"type": "simple", "path": "{}"}}]"#, foo.display()),
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
            format!(r#"[{{"type": "simple", "path": "{}"}}]"#, foo.display()),
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
            format!(r#"[{{"type": "simple", "path": "{}"}}]"#, qux.display()),
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
            format!(r#"[{{"type": "simple", "path": "{}"}}]"#, foo.display()),
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
            format!(
                r#"[{{"type": "simple", "path": "{}", "register_to_asset_manager": false}},
                  {{"type": "simple", "path": "{}"}}]"#,
                foo.display(),
                bar.display()
            ),
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
}
