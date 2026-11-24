//! `AssetService`：右槽资产域的进程内总线服务（change asset-service design D1/D2/D4）。
//!
//! 状态单份归属服务端：`registrations` = 已注册的项目 DTO（真值），
//! `view` = 三态视图（每次落地原子替换）。GUI 侧只持**镜像**（经
//! `asset.tree_changed` tell 推达），渲染时读镜像。
//!
//! ## 单 worker 纪律（design D4 修订）
//!
//! `handle()` **只做同步簿记**；枚举体经 `ServiceCtx::messaging::spawn_task`
//! 送入 runtime，并在其中用 `tokio::task::spawn_blocking` 把同步 fs 挪到
//! 阻塞池 —— 否则唯一的 worker 会被 `std::fs::read_dir` 独占，同层的
//! agent / solution 服务全部排队（spec: Enumeration does not delay other
//! services）。结果经 `asset.enum_done` tell **回投自己**，在 handler 里
//! 落地。

use std::path::PathBuf;
use std::sync::Arc;

use crate::contract::{
    ActionInvoke, AssetRegistration, AssetState, EnumerationDone, OpError, OpResult, RefreshReq,
    TreeChanged, TreeSnapshot, WatchReq, KIND_ACTION, KIND_ENUM_DONE, KIND_GET_STATE,
    KIND_OP_REPLY, KIND_REFRESH, KIND_REGISTER, KIND_TREE_CHANGED, KIND_WATCH,
};
use crate::enumerate::EnumeratorOwner;
use service_core::{
    Address, Envelope, InstanceId, Payload, Route, Service, ServiceCtx, ServiceFuture,
};

/// 资产域服务（driver 单线程顺序 dispatch，同步处理 + 同步回执，
/// 对齐 `SolutionService` 形态）。
pub struct AssetService {
    ctx: ServiceCtx,
    /// 已绑定工作区 — `None` = 注册从未到达（`NoWorkspace` 归因依据）。
    ws_dir: Option<PathBuf>,
    /// 注册集（真值；GUI 经 DTO 提供，服务不认识 `Project`）。
    registrations: Vec<crate::contract::ProjectRegistration>,
    /// 三态视图（design D5：原子落地，不流式）。
    view: AssetState,
    /// 视图代数：**失效入口**（register/refresh）与**每次落地**各 +1 —
    /// 面板以相等比对做 O(1) 失效检测（spec: O(1) when nothing changed）。
    generation: u64,
    /// 树变更订阅者（watch 握手记录；单订阅者，对齐 solution/agent）。
    watcher: Option<InstanceId>,
    /// 在途枚举的代数令牌：被 supersede 的结果回投时 `seq != enum_seq`
    /// ⇒ 丢弃（刷新/重注册 supersede 在途，spec: In-flight enumeration）。
    enum_seq: u64,
    /// 上次注册的结构签名 —— 输入未变则不清场、不重枚举、**不发布**
    /// （spec: Published view reaches subscribers … no view message is
    /// published when inputs have not changed）。
    last_sig: Option<RegSig>,
}

/// 注册集的结构签名（可比较）。
///
/// 过滤集本身**不进签名**：它是 `(is_git, root 内容)` 的函数，而
/// `.gitignore` 变化走显式 ⟳（refresh 不跳过），因此结构未变时跳过是
/// 安全的（spec: Refresh re-reads filter rules）。
type RegSig = Vec<(String, PathBuf, bool, Vec<(String, PathBuf)>)>;

fn signature(regs: &[crate::contract::ProjectRegistration]) -> RegSig {
    fn mods(m: &[crate::contract::ModuleRegistration]) -> Vec<(String, PathBuf)> {
        m.iter()
            .flat_map(|x| {
                std::iter::once((x.name.clone(), x.root.clone())).chain(mods(&x.children))
            })
            .collect()
    }
    regs.iter()
        .map(|p| {
            (
                p.name.clone(),
                p.root.clone(),
                p.is_git,
                mods(&p.modules),
            )
        })
        .collect()
}

impl AssetService {
    pub fn new(ctx: ServiceCtx) -> Self {
        Self {
            ctx,
            ws_dir: None,
            registrations: Vec::new(),
            view: AssetState::Empty,
            generation: 0,
            watcher: None,
            enum_seq: 0,
            last_sig: None,
        }
    }

    /// 当前快照（回执与事件共用）。
    pub fn snapshot(&self) -> TreeSnapshot {
        TreeSnapshot {
            state: self.view.clone(),
            generation: self.generation,
        }
    }

    // ---------- 出站 ----------

    /// 控制回执（仅 ask：`corr` 存在才回，对齐 `SolutionService::reply_to`）。
    fn reply_to(&self, env: &Envelope, result: OpResult) {
        if env.corr.is_some() {
            let _ = self
                .ctx
                .messaging
                .reply(env, KIND_OP_REPLY, Payload::new(result));
        }
    }

    /// 视图变更事件（推 watcher；事件自带快照）。
    fn emit(&self, snap: TreeSnapshot) {
        let Some(watcher) = self.watcher else { return };
        let _ = self.ctx.messaging.tell(Envelope::new(
            Route::Instance(watcher),
            Address::instance(self.ctx.instance),
            KIND_TREE_CHANGED,
            Payload::new(TreeChanged(snap)),
        ));
    }

    /// 失效 +1（不改视图内容）—— register/refresh 入口用。
    fn invalidate(&mut self) {
        self.generation += 1;
    }

    // ---------- 分派 ----------

    fn dispatch(&mut self, env: Envelope) {
        match env.kind {
            KIND_REGISTER => self.on_register(&env),
            KIND_REFRESH => self.on_refresh(&env),
            KIND_GET_STATE => self.on_get_state(&env),
            KIND_ACTION => self.on_action(&env),
            KIND_WATCH => self.on_watch(&env),
            KIND_ENUM_DONE => self.on_enum_done(&env),
            _ => {
                let _ = self.ctx.messaging.reject(&env);
            }
        }
    }

    /// 全量注册（workspace 打开 / 切换 / 注册集变化）。
    ///
    /// - 先清场到 `Empty` 再起枚举（spec: 切换不闪旧树 / no stale rows）
    /// - 零注册 → 停在 `Empty` 且**不跑枚举**（spec: zero-project … no
    ///   enumeration runs）
    fn on_register(&mut self, env: &Envelope) {
        let Ok(req) = env.downcast::<AssetRegistration>().cloned() else {
            let _ = self.ctx.messaging.reject(env);
            return;
        };
        // ---- 输入未变 → 什么都不做（不清场、不重枚举、不发布）----
        // spec: Published view reaches subscribers —— 输入没变时不得推消息。
        let sig = signature(&req.projects);
        if self.ws_dir.as_deref() == Some(req.workspace_dir.as_path())
            && self.last_sig.as_ref() == Some(&sig)
        {
            self.reply_to(env, Ok(self.snapshot()));
            return;
        }
        self.ws_dir = Some(req.workspace_dir);
        self.registrations = req.projects;
        self.last_sig = Some(sig);
        self.invalidate();
        self.view = AssetState::Empty;

        if self.registrations.is_empty() {
            let snap = self.snapshot();
            self.reply_to(env, Ok(snap.clone()));
            self.emit(snap);
            return;
        }
        self.spawn_enumeration();
        let snap = self.snapshot();
        self.reply_to(env, Ok(snap.clone()));
        self.emit(snap);
    }

    /// 显式刷新（⟳）：重读 filter 源 + **supersede 在途**（spec: Explicit
    /// refresh still restarts）。从未注册 ⇒ 结构化 `NoWorkspace`。
    fn on_refresh(&mut self, env: &Envelope) {
        if env.downcast::<RefreshReq>().is_err() {
            let _ = self.ctx.messaging.reject(env);
            return;
        }
        if self.ws_dir.is_none() {
            self.reply_to(env, Err(OpError::NoWorkspace));
            return;
        }
        self.invalidate();
        if self.registrations.is_empty() {
            self.view = AssetState::Empty;
            let snap = self.snapshot();
            self.reply_to(env, Ok(snap.clone()));
            self.emit(snap);
            return;
        }
        self.spawn_enumeration();
        let snap = self.snapshot();
        self.reply_to(env, Ok(snap.clone()));
        self.emit(snap);
    }

    /// 读当前视图（GUI 冷启动补读；正常链路靠 watch 推达）。
    fn on_get_state(&mut self, env: &Envelope) {
        // 无载荷 kind — 只认 corr 形态
        let _ = env;
        self.reply_to(env, Ok(self.snapshot()));
    }

    /// 节点动作写入口单点（`asset.action`）。
    ///
    /// asset 侧两个动作（复制路径 / 在 Finder 显示）均标
    /// `ExecutedBy::View`（design D5）⇒ **服务收到的只能是将来新增的
    /// 服务侧动作**；当前收到一律按「未声明」立即回执（不 reject ——
    /// reject 不回信封，调用方要等 5s 超时才拿到笼统文案）。
    fn on_action(&mut self, env: &Envelope) {
        if self.ws_dir.is_none() {
            self.reply_to(env, Err(OpError::NoWorkspace));
            return;
        }
        let Ok(req) = env.downcast::<ActionInvoke>() else {
            let _ = self.ctx.messaging.reject(env);
            return;
        };
        self.reply_to(env, Err(OpError::UnknownAction(req.action_id.clone())));
    }

    fn on_watch(&mut self, env: &Envelope) {
        if env.downcast::<WatchReq>().is_err() {
            let _ = self.ctx.messaging.reject(env);
            return;
        }
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

    /// 枚举完成回投（`spawn_blocking` 内跑完 → tell 自己）。
    ///
    /// `seq` 不匹配 = 本次已被刷新/重注册 supersede ⇒ **丢弃**，旧结果
    /// 不得覆盖新视图。
    fn on_enum_done(&mut self, env: &Envelope) {
        let Ok(done) = env.downcast::<EnumerationDone>() else {
            let _ = self.ctx.messaging.reject(env);
            return;
        };
        if done.seq != self.enum_seq {
            return; // superseded — 静默丢弃
        }
        self.view = match &done.result {
            Ok(nodes) => AssetState::Forest { nodes: nodes.clone() },
            Err(msg) => AssetState::Degraded {
                message: msg.clone(),
            },
        };
        self.generation += 1; // 落地 +1
        let snap = self.snapshot();
        self.emit(snap);
    }

    // ---------- 枚举 ----------

    /// 起一次枚举：**supersede 在途**（seq 令牌），结果异步回投。
    ///
    /// `handle()` 在此处即返回 —— 阻塞 fs 全部落在 `spawn_blocking`
    /// （design D4 修订），单 worker 只跑簿记。
    fn spawn_enumeration(&mut self) {
        if self.registrations.is_empty() {
            return;
        }
        self.enum_seq += 1; // 旧在途的结果届时 seq 不匹配 → 丢弃
        let seq = self.enum_seq;
        let regs = self.registrations.clone();
        let messaging = Arc::clone(&self.ctx.messaging);
        let me = Route::Instance(self.ctx.instance);
        let from = Address::instance(self.ctx.instance);

        self.ctx.messaging.spawn_task(Box::pin(async move {
            // 同步 fs 必须走阻塞池：runtime 是 worker_threads(1)，
            // 协作式交错扛不住 std::fs（spec: Enumeration does not delay
            // other services）。
            let outcome = tokio::task::spawn_blocking(move || {
                // 同步 fs 全部落在本线程（阻塞池）—— 随回执带回线程 id，
                // 供观测断言它 ≠ 单 worker (design D4)。
                let fs_thread = std::thread::current().id();
                let mut nodes = Vec::with_capacity(regs.len());
                let mut err = None;
                for reg in regs {
                    match reg.enumerator().enumerate() {
                        Ok(n) => nodes.push(n),
                        Err(e) => {
                            err = Some(e);
                            break;
                        }
                    }
                }
                let result = match err {
                    Some(e) => Err(e),
                    None => Ok(nodes),
                };
                (result, fs_thread)
            })
            .await;
            let (result, fs_thread) = match outcome {
                Ok(pair) => (Some(pair.0), Some(pair.1)),
                Err(join) => (
                    Some(Err(format!("enumeration task failed: {join}"))),
                    None,
                ),
            };
            let result = result.expect("join branch always carries a result");
            let _ = messaging.tell(Envelope::new(
                me,
                from,
                KIND_ENUM_DONE,
                Payload::new(EnumerationDone {
                    seq,
                    result,
                    fs_thread,
                }),
            ));
        }));
    }
}

impl Service for AssetService {
    fn start(&mut self) -> Result<(), String> {
        // 无外部依赖要拉起 — 注册由 GUI 随 workspace 到达。
        Ok(())
    }

    fn handle(&mut self, env: Envelope) -> ServiceFuture<'_> {
        self.dispatch(env);
        Box::pin(std::future::ready(()))
    }
}

/// 工厂（对齐 `solution_service_factory`；`ServiceOrigin::Internal` 编译期函数指针）。
pub fn asset_service_factory(ctx: ServiceCtx) -> Box<dyn Service> {
    Box::new(AssetService::new(ctx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{
        AssetKind, AssetNode, ModuleRegistration, ProjectRegistration, ACTION_COPY_PATH,
    };
    use crate::testkit::{ask_env, tell_env, test_ctx};

    /// 本实现的 `handle` **在返回前已同步完成分派**，返回值只是已就绪的
    /// 空 future —— 测试调用后状态即可断言（无需另行驱动）。
    fn drive(svc: &mut AssetService, env: Envelope) {
        drop(svc.handle(env));
    }

    fn node(label: &str) -> AssetNode {
        AssetNode {
            id: format!("/a/{label}"),
            label: label.to_string(),
            kind: AssetKind::File,
            children: Vec::new(),
            actions: crate::contract::view_row_actions(),
        }
    }

    fn reg(root: &str) -> ProjectRegistration {
        ProjectRegistration {
            name: "p".into(),
            root: root.into(),
            is_git: false,
            modules: vec![ModuleRegistration {
                name: "p".into(),
                root: root.into(),
                children: Vec::new(),
            }],
        }
    }

    fn replies(msg: &crate::testkit::TestMessaging) -> Vec<OpResult> {
        msg.sent()
            .iter()
            .filter(|e| e.kind == KIND_OP_REPLY)
            .filter_map(|e| e.payload.downcast::<OpResult>().cloned())
            .collect()
    }

    /// 3.2: 空态起点 + 工厂可用。
    #[test]
    fn starts_empty_and_factory_builds() {
        let (ctx, _msg) = test_ctx(1);
        let mut svc = AssetService::new(ctx);
        assert!(svc.start().is_ok());
        assert_eq!(svc.snapshot().state, AssetState::Empty);
        assert_eq!(svc.snapshot().generation, 0);

        let (ctx2, _m2) = test_ctx(2);
        let mut built = asset_service_factory(ctx2);
        assert!(built.start().is_ok());
    }

    /// 3.2: 零注册 → 停在 Empty 且**不跑枚举**（spec: zero-project … no
    /// enumeration runs）。
    #[test]
    fn register_with_zero_projects_stays_empty_without_enumerating() {
        let (ctx, msg) = test_ctx(3);
        let mut svc = AssetService::new(ctx);
        drive(
            &mut svc,
            ask_env(
                KIND_REGISTER,
                AssetRegistration {
                    workspace_dir: "/ws".into(),
                    projects: Vec::new(),
                },
            ),
        );
        assert_eq!(svc.snapshot().state, AssetState::Empty);
        assert_eq!(msg.dropped_tasks(), 0, "零注册不得起枚举");
        assert_eq!(replies(&msg).len(), 1, "ask 必须回执");
        assert_eq!(svc.snapshot().generation, 1, "失效入口 +1");
    }

    /// 3.2: 有注册 → **交接后台**（handler 不内联跑），回执时视图仍是
    /// 清场后的 Empty（枚举尚未落地）。
    #[test]
    fn register_with_projects_hands_enumeration_off() {
        let (ctx, msg) = test_ctx(4);
        let mut svc = AssetService::new(ctx);
        drive(
            &mut svc,
            ask_env(
                KIND_REGISTER,
                AssetRegistration {
                    workspace_dir: "/ws".into(),
                    projects: vec![reg("/p")],
                },
            ),
        );
        assert_eq!(msg.dropped_tasks(), 1, "枚举交给后台，handler 不内联");
        assert_eq!(svc.snapshot().state, AssetState::Empty, "尚在途");
        assert_eq!(svc.snapshot().generation, 1, "失效入口 +1");
    }

    /// 3.2: 枚举落地 → 三态转 Forest、代数再 +1、推 watcher。
    #[test]
    fn enum_done_lands_forest_and_notifies_watcher() {
        let (ctx, msg) = test_ctx(5);
        let mut svc = AssetService::new(ctx);
        drive(&mut svc, ask_env(KIND_WATCH, crate::contract::WatchReq));
        drive(
            &mut svc,
            ask_env(
                KIND_REGISTER,
                AssetRegistration {
                    workspace_dir: "/ws".into(),
                    projects: vec![reg("/p")],
                },
            ),
        );
        let seq = 1; // spawn_enumeration 首次 +1
        drive(
            &mut svc,
            tell_env(
                KIND_ENUM_DONE,
                EnumerationDone {
                    seq,
                    result: Ok(vec![node("main.rs")]),
                    fs_thread: None,
                },
            ),
        );
        match &svc.snapshot().state {
            AssetState::Forest { nodes } => assert_eq!(nodes.len(), 1),
            other => panic!("应落 Forest, 实为 {other:?}"),
        }
        assert_eq!(svc.snapshot().generation, 2, "落地再 +1");
        let kinds = msg.sent_kinds();
        assert!(
            kinds.contains(&KIND_TREE_CHANGED),
            "视图变更须推 watcher, 实发 {kinds:?}"
        );
    }

    /// 3.3 关键不变量: 被 supersede 的在途枚举回投时**丢弃** —— 旧结果
    /// 不得覆盖新视图（刷新/重注册 supersede 在途）。
    #[test]
    fn stale_enum_done_is_discarded() {
        let (ctx, _msg) = test_ctx(6);
        let mut svc = AssetService::new(ctx);
        drive(
            &mut svc,
            ask_env(
                KIND_REGISTER,
                AssetRegistration {
                    workspace_dir: "/ws".into(),
                    projects: vec![reg("/p")],
                },
            ),
        );
        drive(
            &mut svc,
            tell_env(
                KIND_ENUM_DONE,
                EnumerationDone {
                    seq: 99, // 过期令牌
                    result: Ok(vec![node("stale.rs")]),
                    fs_thread: None,
                },
            ),
        );
        assert_eq!(
            svc.snapshot().state,
            AssetState::Empty,
            "过期结果必须丢弃"
        );
        assert_eq!(svc.snapshot().generation, 1, "丢弃不得动代数");
    }

    /// 3.2: 枚举失败 → 降级态命名原因（spec: Empty and degraded states）。
    #[test]
    fn enum_error_lands_degraded() {
        let (ctx, _msg) = test_ctx(7);
        let mut svc = AssetService::new(ctx);
        drive(
            &mut svc,
            ask_env(
                KIND_REGISTER,
                AssetRegistration {
                    workspace_dir: "/ws".into(),
                    projects: vec![reg("/p")],
                },
            ),
        );
        drive(
            &mut svc,
            tell_env(
                KIND_ENUM_DONE,
                EnumerationDone {
                    seq: 1,
                    result: Err("/gone 不是可读目录".into()),
                    fs_thread: None,
                },
            ),
        );
        assert!(
            matches!(&svc.snapshot().state, AssetState::Degraded { message } if message.contains("/gone")),
            "降级态须命名原因"
        );
    }

    /// 4.6 镜像一致性：换注册集 = **整体替换**——先清场到 Empty（不闪旧树）
    /// 再落新集，旧项目行不得残留。
    #[test]
    fn workspace_switch_replaces_the_view_without_stale_rows() {
        let (ctx, msg) = test_ctx(14);
        let mut svc = AssetService::new(ctx);
        // 订阅者在位 — 否则 emit 直接返回, "推镜像"断言无从谈起
        drive(&mut svc, ask_env(KIND_WATCH, crate::contract::WatchReq));

        // 第一组: 两个项目
        drive(
            &mut svc,
            ask_env(
                KIND_REGISTER,
                AssetRegistration {
                    workspace_dir: "/ws1".into(),
                    projects: vec![reg("/a"), reg("/b")],
                },
            ),
        );
        assert_eq!(svc.snapshot().state, AssetState::Empty, "交接即清场");
        drive(
            &mut svc,
            tell_env(
                KIND_ENUM_DONE,
                EnumerationDone {
                    seq: 1,
                    result: Ok(vec![node("a.rs"), node("b.rs")]),
                    fs_thread: None,
                },
            ),
        );
        assert!(
            matches!(svc.snapshot().state, AssetState::Forest { nodes } if nodes.len() == 2),
            "第一组落两行"
        );

        // 换工作区: 一个不同项目
        drive(
            &mut svc,
            ask_env(
                KIND_REGISTER,
                AssetRegistration {
                    workspace_dir: "/ws2".into(),
                    projects: vec![reg("/c")],
                },
            ),
        );
        assert_eq!(
            svc.snapshot().state,
            AssetState::Empty,
            "换注册集必须先清场 — 不得闪旧树 (spec: no stale rows)"
        );
        // 在途的旧枚举回投应被 seq 令牌丢弃
        drive(
            &mut svc,
            tell_env(
                KIND_ENUM_DONE,
                EnumerationDone {
                    seq: 1, // 过期 (第二次 register 已把 enum_seq 推到 2)
                    result: Ok(vec![node("a.rs"), node("b.rs")]),
                    fs_thread: None,
                },
            ),
        );
        assert_eq!(
            svc.snapshot().state,
            AssetState::Empty,
            "过期回投不得把旧树写回"
        );
        drive(
            &mut svc,
            tell_env(
                KIND_ENUM_DONE,
                EnumerationDone {
                    seq: 2,
                    result: Ok(vec![node("c.rs")]),
                    fs_thread: None,
                },
            ),
        );
        match &svc.snapshot().state {
            AssetState::Forest { nodes } => {
                assert_eq!(nodes.len(), 1, "只剩新注册集");
                assert_eq!(nodes[0].label, "c.rs", "内容为新工作区的产物");
            }
            other => panic!("应落新集的 Forest, 实为 {other:?}"),
        }
        assert!(
            msg.sent_kinds()
                .iter()
                .filter(|k| **k == KIND_TREE_CHANGED)
                .count()
                >= 2,
            "每次落地都推镜像"
        );
    }

    /// spec: Explicit refresh still restarts —— 在途期间 ⟳ 仍起新一轮
    /// （supersede：seq 令牌推进，旧结果届时被丢）。
    #[test]
    fn explicit_refresh_restarts_inflight_enumeration() {
        let (ctx, msg) = test_ctx(13);
        let mut svc = AssetService::new(ctx);
        drive(
            &mut svc,
            ask_env(
                KIND_REGISTER,
                AssetRegistration {
                    workspace_dir: "/ws".into(),
                    projects: vec![reg("/p")],
                },
            ),
        );
        assert_eq!(msg.dropped_tasks(), 1, "注册起第一轮");
        drive(&mut svc, ask_env(KIND_REFRESH, RefreshReq));
        assert_eq!(
            msg.dropped_tasks(),
            2,
            "⟳ 在途期间仍起新轮 (spec: Explicit refresh still restarts)"
        );
    }

    /// 3.5: 未注册就刷新 → 结构化 `NoWorkspace`（不是 reject —— reject 不
    /// 回信封，调用方要等 5s 超时）。
    #[test]
    fn refresh_before_register_replies_structured_error() {
        let (ctx, msg) = test_ctx(8);
        let mut svc = AssetService::new(ctx);
        drive(&mut svc, ask_env(KIND_REFRESH, RefreshReq));
        let got = replies(&msg);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0], Err(OpError::NoWorkspace));
    }

    /// 3.5: 服务未声明的动作 id → **立即**回 `UnknownAction`（对齐
    /// solution 侧实施期定案 C）。
    #[test]
    fn unknown_action_replies_immediately() {
        let (ctx, msg) = test_ctx(9);
        let mut svc = AssetService::new(ctx);
        drive(
            &mut svc,
            ask_env(
                KIND_REGISTER,
                AssetRegistration {
                    workspace_dir: "/ws".into(),
                    projects: Vec::new(),
                },
            ),
        );
        drive(
            &mut svc,
            ask_env(
                KIND_ACTION,
                ActionInvoke {
                    node: Some("/p/main.rs".into()),
                    action_id: ACTION_COPY_PATH.into(),
                },
            ),
        );
        let got = replies(&msg);
        assert_eq!(
            got.last(),
            Some(&Err(OpError::UnknownAction(ACTION_COPY_PATH.into()))),
            "当前两个 asset 动作都归视图执行 ⇒ 服务侧动作一律未声明"
        );
    }

    /// 3.5: 未知 kind → reject（落死信，调用方不被静默吞）。
    #[test]
    fn unknown_kind_is_rejected() {
        let (ctx, msg) = test_ctx(10);
        let mut svc = AssetService::new(ctx);
        drive(&mut svc, tell_env("asset.bogus", ()));
        assert_eq!(msg.sent_kinds(), vec!["asset.bogus"], "reject 回投原信封");
    }

    fn tree_changed_count(msg: &crate::testkit::TestMessaging) -> usize {
        msg.sent_kinds()
            .iter()
            .filter(|k| **k == KIND_TREE_CHANGED)
            .count()
    }

    /// 4.1: 落地**恰好推一条**（事件自带整视图快照，不流式）。
    #[test]
    fn landing_publishes_exactly_one_message() {
        let (ctx, msg) = test_ctx(11);
        let mut svc = AssetService::new(ctx);
        drive(&mut svc, ask_env(KIND_WATCH, crate::contract::WatchReq));
        drive(
            &mut svc,
            ask_env(
                KIND_REGISTER,
                AssetRegistration {
                    workspace_dir: "/ws".into(),
                    projects: vec![reg("/p")],
                },
            ),
        );
        let before = tree_changed_count(&msg);
        drive(
            &mut svc,
            tell_env(
                KIND_ENUM_DONE,
                EnumerationDone {
                    seq: 1,
                    result: Ok(vec![node("main.rs")]),
                    fs_thread: None,
                },
            ),
        );
        assert_eq!(
            tree_changed_count(&msg),
            before + 1,
            "一次落地只推一条"
        );
        assert_eq!(svc.snapshot().generation, 2, "失效 +1、落地 +1");
    }

    /// 4.1: **输入未变 → 不推**（不清场、不重枚举、不动代数）——
    /// spec: Published view reaches subscribers … no view message is
    /// published when inputs have not changed.
    #[test]
    fn unchanged_registration_publishes_nothing() {
        let (ctx, msg) = test_ctx(12);
        let mut svc = AssetService::new(ctx);
        drive(&mut svc, ask_env(KIND_WATCH, crate::contract::WatchReq));
        let payload = AssetRegistration {
            workspace_dir: "/ws".into(),
            projects: vec![reg("/p")],
        };
        drive(&mut svc, ask_env(KIND_REGISTER, payload.clone()));

        let trees = tree_changed_count(&msg);
        let spawned = msg.dropped_tasks();
        let generation = svc.snapshot().generation;
        let state = svc.snapshot().state.clone();

        // 同一输入重发
        drive(&mut svc, ask_env(KIND_REGISTER, payload));

        assert_eq!(tree_changed_count(&msg), trees, "输入未变不得发布");
        assert_eq!(msg.dropped_tasks(), spawned, "不得重起枚举");
        assert_eq!(svc.snapshot().generation, generation, "不得失效");
        assert_eq!(svc.snapshot().state, state, "不得清场");
        assert_eq!(
            replies(&msg).len(),
            2,
            "两次 ask 都必须有回执（跳过不等于不回）"
        );
    }

}
