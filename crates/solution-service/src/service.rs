//! `SolutionService`：Solution/Project 域的进程内总线服务（design D1/D7/D8）。
//!
//! 状态单份归属服务端：`items` = 已加载的 solution 配置真值；项目树/交树
//! 快照由 [`Self::snapshot`] 现算（is_git 解析 + 注册门控）。WorkTree 管理
//! 后续挂接（本 change 只留口子，不建空壳）。

use crate::contract::{
    ConfigureReq, CreateReq, GetTreeReq, LoadReq, OpError, OpResult, ProjectTree, SolutionItem,
    TreeChanged, TreeSnapshot, WatchReq, KIND_CONFIGURE, KIND_CREATE, KIND_GET_TREE, KIND_LOAD,
    KIND_OP_REPLY, KIND_TREE_CHANGED, KIND_WATCH,
};
use crate::loader;
use crate::valid_checker;
use service_core::{
    Address, Envelope, Payload, Route, Service, ServiceCtx, ServiceFuture,
};

/// Solution/Project 域服务（driver 单线程顺序 dispatch，同步处理 + 同步回执，
/// 对齐 `AgentService` 形态）。
pub struct SolutionService {
    ctx: ServiceCtx,
    /// 状态单份（design D7）：solution items 的真值（视图经快照镜像读取）。
    items: Vec<SolutionItem>,
    /// 树变更订阅者（watch 握手记录；单订阅者，对齐 AgentService）。
    watcher: Option<service_core::InstanceId>,
}

impl SolutionService {
    pub fn new(ctx: ServiceCtx) -> Self {
        Self {
            ctx,
            items: Vec::new(),
            watcher: None,
        }
    }

    /// 由当前状态现算快照（spec: Asset registration flag gates project
    /// handoff —— `handoff` 只含注册开关开启的 solution 的 project；
    /// `tree` 为全量集，注册关闭的项目仍属于 solution 的 project 集）。
    pub fn snapshot(&self) -> TreeSnapshot {
        let mut tree = Vec::with_capacity(self.items.len());
        let mut handoff = Vec::new();
        for item in &self.items {
            let desc = loader::expand_item(item);
            if item.register_to_asset_manager() {
                handoff.push(desc.clone());
            }
            tree.push(desc);
        }
        TreeSnapshot {
            solutions: self.items.clone().into(),
            tree: ProjectTree {
                projects: tree.into(),
            },
            handoff: ProjectTree {
                projects: handoff.into(),
            },
        }
    }

    /// 信封分发（driver 顺序上下文；未知 kind 拒收落死信，对齐 AgentService）。
    fn dispatch(&mut self, env: Envelope) {
        match env.kind {
            KIND_LOAD => self.on_load(&env),
            KIND_GET_TREE => self.on_get_tree(&env),
            KIND_CREATE => self.on_create(&env),
            KIND_CONFIGURE => self.on_configure(&env),
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

    /// `create`：append simple item + 落盘。幂等（spec: Create idempotent）：
    /// 同 path 已存在 → 原样回执、不重复。写失败 → `Persist` 红条回执 +
    /// **内存变更保留** + 树变更照发（workspace-management 契约）。
    fn on_create(&mut self, env: &Envelope) {
        let Ok(req) = env.downcast::<CreateReq>() else {
            let _ = self.ctx.messaging.reject(env);
            return;
        };
        if valid_checker::create_is_noop(&self.items, &req.path) {
            // 幂等 no-op (spec: Re-adding an existing solution changes
            // nothing): 不写盘、不发树变更、原样回执
            self.reply_to(env, Ok(self.snapshot()));
            return;
        }
        self.items.push(SolutionItem::Simple {
            path: req.path.clone(),
            register_to_asset_manager: true,
            git_enabled: true,
        });
        let snap = self.snapshot();
        match loader::save_items(&req.workspace_dir, &self.items) {
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
    fn on_configure(&mut self, env: &Envelope) {
        let Ok(req) = env.downcast::<ConfigureReq>() else {
            let _ = self.ctx.messaging.reject(env);
            return;
        };
        if !self.items.iter().any(|i| i.path() == req.solution_path.as_path()) {
            self.reply_to(env, Err(OpError::NoSuch(req.solution_path.clone())));
            return;
        }
        if valid_checker::configure_is_noop(
            &self.items,
            &req.solution_path,
            req.register_to_asset_manager,
            req.git_enabled,
        ) {
            // 幂等 no-op: 状态/文件/交树均不变 (spec), 不发树变更
            self.reply_to(env, Ok(self.snapshot()));
            return;
        }
        for item in &mut self.items {
            if item.path() != req.solution_path.as_path() {
                continue;
            }
            if let Some(v) = req.register_to_asset_manager {
                item.set_register_to_asset_manager(v);
            }
            if let Some(v) = req.git_enabled {
                item.set_git_enabled(v);
            }
        }
        let snap = self.snapshot();
        match loader::save_items(&req.workspace_dir, &self.items) {
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
    use crate::contract::LoadError;
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
        assert_eq!(snap.tree.projects.len(), 1);
        assert_eq!(snap.tree.projects[0].name, "foo");
        assert_eq!(snap.handoff.projects.len(), 1);

        drive(&mut svc, ask_env(KIND_GET_TREE, GetTreeReq));
        assert_eq!(last_reply(&msg), Ok(snap), "GetTree 与 Load 回执同源同值");
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
        assert_eq!(last_reply(&msg).unwrap().tree.projects.len(), 1);

        // create 增量
        drive(&mut svc, ask_env(
            KIND_CREATE,
            CreateReq {
                workspace_dir: ws.clone(),
                path: bar.clone(),
            },
        ));
        assert_eq!(last_reply(&msg).unwrap().tree.projects.len(), 2);

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
        assert_eq!(snap.tree.projects.len(), 1, "load 替换而非追加");
        assert_eq!(snap.tree.projects[0].name, "qux");
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

        drive(&mut svc, ask_env(
            KIND_CREATE,
            CreateReq {
                workspace_dir: ws.clone(),
                path: foo.clone(),
            },
        ));
        let first = last_reply(&msg).unwrap();
        assert_eq!(first.tree.projects.len(), 1);
        let tells_after_first = msg
            .sent()
            .iter()
            .filter(|e| e.kind == KIND_TREE_CHANGED)
            .count();
        assert!(tells_after_first >= 1, "首次 create 应发树变更");

        // 重复 create → 幂等: 不重复、不写盘、不再发树变更
        drive(&mut svc, ask_env(
            KIND_CREATE,
            CreateReq {
                workspace_dir: ws.clone(),
                path: foo.clone(),
            },
        ));
        let second = last_reply(&msg).unwrap();
        assert_eq!(second.tree.projects.len(), 1, "不产生重复 project");
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
        let cfg = ConfigureReq {
            workspace_dir: ws.clone(),
            solution_path: foo.clone(),
            register_to_asset_manager: Some(false),
            git_enabled: Some(false),
        };
        drive(&mut svc, ask_env(KIND_CONFIGURE, cfg));
        let after_change = last_reply(&msg).unwrap();
        assert!(after_change.handoff.projects.is_empty(), "关注册后交树收缩");
        let tells_after_change = msg
            .sent()
            .iter()
            .filter(|e| e.kind == KIND_TREE_CHANGED)
            .count();

        // 同值重放 → no-op: 状态不变、不再发树变更
        let cfg2 = ConfigureReq {
            workspace_dir: ws.clone(),
            solution_path: foo.clone(),
            register_to_asset_manager: Some(false),
            git_enabled: Some(false),
        };
        drive(&mut svc, ask_env(KIND_CONFIGURE, cfg2));
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
        assert_eq!(snap.tree.projects.len(), 2, "全量集含双项目 (solution 自己的 project 集)");
        assert_eq!(snap.handoff.projects.len(), 1, "交树只含注册开启的");
        assert_eq!(snap.handoff.projects[0].name, "bar");

        // scenario 2: Enabling mid-session registers on next handoff (不重启)
        drive(&mut svc, ask_env(
            KIND_CONFIGURE,
            ConfigureReq {
                workspace_dir: ws.clone(),
                solution_path: foo.clone(),
                register_to_asset_manager: Some(true),
                git_enabled: None,
            },
        ));
        let snap = last_reply(&msg).unwrap();
        assert_eq!(snap.handoff.projects.len(), 2, "enable 后下次交树出现 (不重启)");
    }
}
