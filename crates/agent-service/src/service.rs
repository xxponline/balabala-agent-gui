//! `AgentService`：进程内会话薄服务（design D1/D2/D3）。
//!
//! task 1.2 = 骨架；2.1 = 会话注册表 + 三态状态观测；
//! 2.2-2.6 递进接入控制/输入/观察/投影/drain。

use crate::contract::{
    CloseOk, ControlErr, ControlResult, CreateOk, FrameBytes, InputClick, InputKey, InputText,
    InputWheel, KIND_CLOSE, KIND_CONTROL_REPLY, KIND_CREATE, KIND_FRAME_BYTES, KIND_INPUT_CLICK,
    KIND_INPUT_KEY, KIND_INPUT_TEXT, KIND_INPUT_WHEEL, KIND_RESIZE, KIND_SELECT, KIND_THEME,
    KIND_WATCH, ResizeReq, SelectOk, ThemeReq, WatchReq,
};
use crate::contract::{SessionId, SessionInfo, SessionStatus, SessionView};
use crate::session::{SessionFactory, SessionHandle, SessionIo};
use pi_shell_rs::app::theme::TuiTheme;
use service_core::{
    Address, DeadLetterReason, Envelope, InstanceId, Payload, RouteError, Service, ServiceCtx,
    ServiceFuture, TaskFuture,
};
use std::any::Any;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// drain 循环拍距 (design D5: 与既有 50ms 宿主泵同相位)。
const DRAIN_INTERVAL_MS: u64 = 50;
/// 循环自寻址 kind (service 内部, GUI 不发)。
const KIND_DRAIN_TICK: &str = "drain.tick";
/// 循环自寻址载荷 (空; dispatch 按 kind 分发)。
struct DrainTick;

/// 一次 drain tick 的产出：状态是否变化 + 各会话新字节（`Arc` 一次构造，
/// 推帧时克隆引用 —— 零拷贝，D3）。
#[allow(dead_code)] // task 2.6 drain 循环调用；帧已在 tick 内推送
pub(crate) struct TickOutcome {
    pub changed: bool,
    pub frames: Vec<(SessionId, Arc<Vec<u8>>)>,
}

/// 服务实例状态。
#[allow(dead_code)] // factory/next_id/watcher/theme/dropped_frames 随 task 2.2-2.6 handler 接线读取
pub struct AgentService {
    ctx: ServiceCtx,
    factory: SessionFactory,
    /// 注册表（driver 单线程顺序访问；规模 = 会话数，线性足够）。
    sessions: Vec<SessionHandle>,
    next_id: u64,
    /// active 归服务唯一权威（design D4）。
    active: Option<SessionId>,
    /// watch 握手存下的订阅者地址（2.4/2.5 推送目标）。
    watcher: Option<InstanceId>,
    /// 最近 `ThemeReq` 快照（create 注入 / theme 广播基线）。
    theme: TuiTheme,
    /// per-project `pi-N` 名额计数（spec: N 每项目从 1 起、**永不复用**——
    /// 计数存服务侧，关闭不回退；创建失败也已消耗，与原 claim-then-spawn 一致）。
    counters: HashMap<PathBuf, u64>,
    /// 推帧失败（mailbox 满）丢帧计数（观测面，D3）。
    dropped_frames: u64,
}

impl AgentService {
    /// 丢帧计数（观测面：出口满被拒的帧数）。
    pub fn dropped_frames(&self) -> u64 {
        self.dropped_frames
    }
}

impl AgentService {
    pub fn new(ctx: ServiceCtx) -> Self {
        Self::with_factory(ctx, default_factory())
    }

    pub fn with_factory(ctx: ServiceCtx, factory: SessionFactory) -> Self {
        Self {
            ctx,
            factory,
            sessions: Vec::new(),
            next_id: 0,
            active: None,
            watcher: None,
            theme: TuiTheme::default(),
            counters: HashMap::new(),
            dropped_frames: 0,
        }
    }

    /// 本服务实例 id（观测/测试用）。
    pub fn instance_id(&self) -> InstanceId {
        self.ctx.instance
    }

    // ---------- task 2.1: 注册表 ----------

    /// 登记新会话（`create` handler 的底层；测试直插用）。
    #[allow(dead_code)] // task 2.2 create handler 接线
    fn insert_session(&mut self, name: String, root: PathBuf, io: Box<dyn SessionIo>) -> SessionId {
        self.next_id += 1;
        let id = SessionId(self.next_id);
        self.sessions.push(SessionHandle {
            id,
            name,
            root,
            status: SessionStatus::Starting,
            size: None,
            io,
        });
        id
    }

    /// 指定会话的状态（测试/投影断言）。
    pub fn session_status(&self, id: SessionId) -> Option<SessionStatus> {
        self.sessions.iter().find(|h| h.id == id).map(|h| h.status)
    }

    /// 当前 active（D4 唯一权威）。
    pub fn active(&self) -> Option<SessionId> {
        self.active
    }

    /// 全量快照（投影事件载荷 / 测试断言）。
    pub fn session_view(&self) -> SessionView {
        SessionView {
            sessions: self
                .sessions
                .iter()
                .map(|h| SessionInfo {
                    id: h.id,
                    name: h.name.clone(),
                    project_root: h.root.clone(),
                    status: h.status,
                })
                .collect::<Vec<_>>()
                .into(),
            active: self.active,
        }
    }

    // ---------- task 2.1: drain + 三态状态观测 ----------

    /// 一次 drain tick：各会话取字节（所有权移动）+ 状态迁移 + 帧全推。
    ///
    /// - `Starting` + 首批非空字节 → `Running`
    /// - io 终止（`!alive()`）→ `Exited`（残留字节仍作为最后一帧产出）
    /// - 已 watch 则全会话推 `frame_bytes`（每帧 `Arc` 克隆；推失败丢帧计数）
    #[allow(dead_code)] // task 2.6 drain 循环接线
    pub(crate) fn tick(&mut self) -> TickOutcome {
        let mut changed = false;
        let mut frames = Vec::new();
        for h in &mut self.sessions {
            let bytes = h.io.drain();
            if !h.io.alive() && h.status != SessionStatus::Exited {
                h.status = SessionStatus::Exited;
                changed = true;
            } else if !bytes.is_empty() && h.status == SessionStatus::Starting {
                h.status = SessionStatus::Running;
                changed = true;
            }
            if !bytes.is_empty() {
                frames.push((h.id, Arc::new(bytes)));
            }
        }
        // 帧全推 (task 2.4): watch 前零推; 任一会话有字节即各自成帧
        if let Some(watcher) = self.watcher {
            for (session, bytes) in &frames {
                let env = Envelope::new(
                    service_core::Route::Instance(watcher),
                    Address::instance(self.ctx.instance),
                    KIND_FRAME_BYTES,
                    Payload::new(FrameBytes {
                        session: *session,
                        bytes: Arc::clone(bytes),
                    }),
                );
                if self.ctx.messaging.tell(env).is_err() {
                    self.dropped_frames += 1; // 出口满: 丢帧自愈, 不阻塞 drain
                }
            }
        }
        // 状态变化 → 投影事件 (task 2.5)
        if changed {
            self.emit_sessions();
        }
        TickOutcome { changed, frames }
    }

    /// 50ms drain 循环（design D5）：宿主 runtime 上定时**自寻址** ——
    /// 循环只发 `drain.tick` 信封给本实例，tick 执行仍在 driver 顺序上下文
    /// （`&mut self` 不跨线程，无锁）。首 tick 是 interval 的立即触发，
    /// 先跳过（此刻 start 尚未转 available，自投会被 fail-fast 拒）。
    fn spawn_drain_loop(&self) {
        let messaging = Arc::clone(&self.ctx.messaging);
        let id = self.ctx.instance;
        let fut: TaskFuture = Box::pin(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(DRAIN_INTERVAL_MS));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            interval.tick().await; // 跳过立即首 tick
            loop {
                interval.tick().await;
                let env = Envelope::new(
                    service_core::Route::Instance(id),
                    Address::instance(id),
                    KIND_DRAIN_TICK,
                    Payload::new(DrainTick),
                );
                match messaging.tell(env) {
                    Ok(()) => {}
                    // 瞬时出口满: 不杀循环, 下拍再试 (帧自含状态, 丢拍无害)
                    Err(RouteError::DeadLetter(DeadLetterReason::ChannelFull)) => continue,
                    // 实例不可达 (stopping): 循环自退出
                    Err(_) => break,
                }
            }
        });
        self.ctx.messaging.spawn_task(fut);
    }

    /// 输入后的补采拍距 (design D5 回改): 本地回显帧在输入后 ~1-2ms 就绪,
    /// 主 tick 50ms 相位会把"输入→帧"拖到 ~50ms (实测 48-52ms) — 与 GUI
    /// 旧 burst (8ms×4) 同款三拍补采压回 burst 档 (实测见 design D5)。
    const INPUT_BURST_STEPS: usize = 3;
    const INPUT_BURST_STEP_MS: u64 = 8;

    /// 输入后快采: 三拍 (8/16/24ms 真实时钟) 自寻址 `drain.tick`。
    /// 与主 tick 同款退出语义: 出口满 → 丢拍继续; 实例不可达 → 自退出。
    fn spawn_input_burst(&self) {
        let messaging = Arc::clone(&self.ctx.messaging);
        let id = self.ctx.instance;
        let fut: TaskFuture = Box::pin(async move {
            for _ in 0..Self::INPUT_BURST_STEPS {
                tokio::time::sleep(Duration::from_millis(Self::INPUT_BURST_STEP_MS)).await;
                let env = Envelope::new(
                    service_core::Route::Instance(id),
                    Address::instance(id),
                    KIND_DRAIN_TICK,
                    Payload::new(DrainTick),
                );
                match messaging.tell(env) {
                    Ok(()) => {}
                    Err(RouteError::DeadLetter(DeadLetterReason::ChannelFull)) => continue,
                    Err(_) => break,
                }
            }
        });
        self.ctx.messaging.spawn_task(fut);
    }

    /// `watch` 握手：订阅身份取信封 `from`（D5 受限地址恰为此设，D3）。
    fn on_watch(&mut self, env: &Envelope) {
        let Ok(_) = env.downcast::<WatchReq>() else {
            let _ = self.ctx.messaging.reject(env);
            return;
        };
        match *env.from.route() {
            service_core::Route::Instance(id) => {
                self.watcher = Some(id);
                // 首次订阅即推全量快照 (GUI 镜像冷启动零回程)
                self.emit_sessions();
            }
            _ => {
                let _ = self.ctx.messaging.reject(env);
            }
        }
    }

    /// 投影事件：注册表/状态/active 任何变化 → 全量快照推 watcher
    /// （事件自带快照，D10 零回程）。
    fn emit_sessions(&self) {
        let Some(watcher) = self.watcher else {
            return;
        };
        let _ = self.ctx.messaging.tell(Envelope::new(
            service_core::Route::Instance(watcher),
            Address::instance(self.ctx.instance),
            crate::contract::KIND_SESSIONS_CHANGED,
            Payload::new(crate::contract::SessionsChanged(self.session_view())),
        ));
    }
}

/// 默认工厂：会话 spawn 尚未接线（生产工厂由 change task 3.2 注入）。
fn default_factory() -> SessionFactory {
    Arc::new(|_, _, _| Err("agent: session spawn 未接线 (change task 3.2)".into()))
}

impl Service for AgentService {
    fn start(&mut self) -> Result<(), String> {
        // task 2.6: 装载即起 drain 循环 (宿主 runtime, spawn_task 代跑)
        self.spawn_drain_loop();
        Ok(())
    }

    fn handle(&mut self, env: Envelope) -> ServiceFuture<'_> {
        self.dispatch(env);
        Box::pin(std::future::ready(()))
    }
}

impl AgentService {
    /// 信封分发（driver 单线程顺序调用；同步处理 + 同步回执，CoreFake 同款形状）。
    fn dispatch(&mut self, env: Envelope) {
        // 输入四类: 转发后内联 drain (design D5 — 输入即突发, 立即成帧)
        let inline_drain = matches!(
            env.kind,
            KIND_INPUT_KEY | KIND_INPUT_TEXT | KIND_INPUT_WHEEL | KIND_INPUT_CLICK
        );
        match env.kind {
            KIND_SELECT => self.on_select(&env),
            KIND_CREATE => self.on_create(&env),
            KIND_CLOSE => self.on_close(&env),
            // task 2.3: 输入/尺寸/主题 (tell, 静默转发; 未知目标拒收落死信)
            KIND_INPUT_KEY => self.on_input_key(&env),
            KIND_INPUT_TEXT => self.on_input_text(&env),
            KIND_INPUT_WHEEL => self.on_input_wheel(&env),
            KIND_INPUT_CLICK => self.on_input_click(&env),
            KIND_RESIZE => self.on_resize(&env),
            KIND_THEME => self.on_theme(&env),
            KIND_WATCH => self.on_watch(&env),
            // task 2.6: drain 循环自寻址 — tick 执行仍在 driver 顺序上下文
            KIND_DRAIN_TICK => {
                self.tick();
            }
            // 未知 kind → 拒收落死信 (spec R1)
            _ => {
                let _ = self.ctx.messaging.reject(&env);
            }
        }
        if inline_drain {
            self.tick(); // 输入即推帧 (已有字节)
            self.spawn_input_burst(); // 回显帧快采 (design D5 回改)
        }
    }

    /// 控制回执（仅 ask：`corr` 存在才回，D6）。
    fn reply_to<T: Any + Send + Sync>(&self, env: &Envelope, result: ControlResult<T>) {
        if env.corr.is_some() {
            let _ = self
                .ctx
                .messaging
                .reply(env, KIND_CONTROL_REPLY, Payload::new(result));
        }
    }

    /// `select`：id 存在 → active 归属（D4 唯一权威）；未知 id → Err 回执。
    fn on_select(&mut self, env: &Envelope) {
        let Ok(req) = env.downcast::<crate::contract::SelectReq>() else {
            let _ = self.ctx.messaging.reject(env);
            return;
        };
        let id = req.0;
        if self.sessions.iter().any(|h| h.id == id) {
            let changed = self.active != Some(id);
            self.active = Some(id);
            self.reply_to(env, Ok(SelectOk));
            if changed {
                self.emit_sessions(); // active 变化 → 投影 (task 2.5)
            }
        } else {
            self.reply_to::<SelectOk>(env, Err(ControlErr(format!("no such session: {}", id.0))));
        }
    }

    /// `create`：名额计数 → 工厂产会话 → 登记 + 置 active；工厂 Err →
    /// 无新行、无投影、Err 回执（spec delta 场景 3）。
    fn on_create(&mut self, env: &Envelope) {
        let Ok(req) = env.downcast::<crate::contract::CreateReq>() else {
            let _ = self.ctx.messaging.reject(env);
            return;
        };
        let root = req.root.clone();
        let n = self.counters.entry(root.clone()).or_insert(0);
        *n += 1;
        // 显示名 = `<project>/pi-N` (project name = root 末段 — spec:
        // Project session creation: 行显示 `foo/pi-1`)
        let proj = root
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "/".to_string());
        let name = format!("{proj}/pi-{n}");
        match (self.factory)(name.clone(), root.clone(), self.theme.clone()) {
            Ok(io) => {
                let id = self.insert_session(name, root, io);
                self.active = Some(id); // spec: 新建即 active
                self.reply_to(env, Ok(CreateOk(id)));
                self.emit_sessions(); // 注册表 + active 变化 (task 2.5)
            }
            Err(e) => self.reply_to::<CreateOk>(env, Err(ControlErr(e))),
        }
    }

    /// `close`：关 io + 移除；关的是 active → 退位剩余最后一个/无则 None
    /// （spec: the next remaining session）；未知/重复 id → 幂等 Err。
    fn on_close(&mut self, env: &Envelope) {
        let Ok(req) = env.downcast::<crate::contract::CloseReq>() else {
            let _ = self.ctx.messaging.reject(env);
            return;
        };
        let id = req.0;
        let Some(idx) = self.sessions.iter().position(|h| h.id == id) else {
            self.reply_to::<CloseOk>(env, Err(ControlErr(format!("no such session: {}", id.0))));
            return;
        };
        let mut handle = self.sessions.remove(idx);
        handle.io.shutdown();
        if self.active == Some(id) {
            self.active = self.sessions.last().map(|h| h.id);
        }
        self.reply_to(env, Ok(CloseOk));
        self.emit_sessions(); // 注册表 (+ active退位) 变化 (task 2.5)
    }

    /// 按 id 取可变 io（输入类转发共用）。
    fn session_io_mut(&mut self, id: SessionId) -> Option<&mut Box<dyn SessionIo>> {
        self.sessions.iter_mut().find(|h| h.id == id).map(|h| &mut h.io)
    }

    /// 输入四类 tell 转发（D6 静默；目标不存在 = 并发关闭竞态 → 拒收可见）。
    fn on_input_key(&mut self, env: &Envelope) {
        let Ok(req) = env.downcast::<InputKey>() else {
            let _ = self.ctx.messaging.reject(env);
            return;
        };
        let (session, key) = (req.session, req.key);
        match self.session_io_mut(session) {
            Some(io) => io.send_key(key),
            None => {
                let _ = self.ctx.messaging.reject(env);
            }
        }
    }

    fn on_input_text(&mut self, env: &Envelope) {
        let Ok(req) = env.downcast::<InputText>() else {
            let _ = self.ctx.messaging.reject(env);
            return;
        };
        let (session, text) = (req.session, req.text.clone());
        match self.session_io_mut(session) {
            Some(io) => io.send_text(&text),
            None => {
                let _ = self.ctx.messaging.reject(env);
            }
        }
    }

    fn on_input_wheel(&mut self, env: &Envelope) {
        let Ok(req) = env.downcast::<InputWheel>() else {
            let _ = self.ctx.messaging.reject(env);
            return;
        };
        let (session, lines) = (req.session, req.lines);
        match self.session_io_mut(session) {
            Some(io) => io.send_wheel(lines),
            None => {
                let _ = self.ctx.messaging.reject(env);
            }
        }
    }

    fn on_input_click(&mut self, env: &Envelope) {
        let Ok(req) = env.downcast::<InputClick>() else {
            let _ = self.ctx.messaging.reject(env);
            return;
        };
        let (session, x, y) = (req.session, req.x, req.y);
        match self.session_io_mut(session) {
            Some(io) => io.send_click(x, y),
            None => {
                let _ = self.ctx.messaging.reject(env);
            }
        }
    }

    /// `resize`：服务侧 `(cols, rows)` 去重后再转 embed（D3 缓存行）。
    fn on_resize(&mut self, env: &Envelope) {
        let Ok(req) = env.downcast::<ResizeReq>() else {
            let _ = self.ctx.messaging.reject(env);
            return;
        };
        let Some(h) = self.sessions.iter_mut().find(|h| h.id == req.session) else {
            let _ = self.ctx.messaging.reject(env);
            return;
        };
        let target = (req.cols, req.rows);
        if h.size != Some(target) {
            h.size = Some(target);
            h.io.resize(req.cols, req.rows);
        }
    }

    /// `theme`：更新服务快照（后续 create 注入）+ 广播全部会话。
    fn on_theme(&mut self, env: &Envelope) {
        let Ok(req) = env.downcast::<ThemeReq>() else {
            let _ = self.ctx.messaging.reject(env);
            return;
        };
        self.theme = req.theme.clone();
        for h in &mut self.sessions {
            h.io.set_theme(&self.theme);
        }
    }
}

/// 内部服务工厂（`ServiceOrigin::Internal`；GUI bootstrap
/// `register_internal("agent", agent_service_factory)` 装载）。
pub fn agent_service_factory(ctx: ServiceCtx) -> Box<dyn Service> {
    Box::new(AgentService::new(ctx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{CloseReq, CreateReq, SelectReq};
    use crate::testkit::{stub_factory, test_ctx, test_ctx_with, StubScript, StubSession, TestMessaging};
    use service_core::{Address, Corr, Envelope, Route};
    use std::sync::Mutex;

    /// 构造 ask 形态的控制信封（to 任意 — dispatch 是直调不查路由）。
    fn ctrl_env(kind: &'static str, payload: Payload, corr: u64) -> Envelope {
        Envelope::new(
            Route::Instance(InstanceId(1)),
            Address::instance(InstanceId(99)),
            kind,
            payload,
        )
        .with_corr(Corr(corr))
    }

    /// 取指定 corr 的控制回执并按类型拆。
    fn last_reply<T: Any + Send + Sync + Clone + std::fmt::Debug>(
        msg: &TestMessaging,
        corr: u64,
    ) -> ControlResult<T> {
        let envs = msg.sent();
        let env = envs
            .iter()
            .find(|e| e.kind == KIND_CONTROL_REPLY && e.corr == Some(Corr(corr)))
            .expect("回执必须存在");
        env.downcast::<ControlResult<T>>()
            .expect("回执载荷类型匹配")
            .clone()
    }

    /// task 1.2 验证: 空注册表 start 返回 Ok。
    #[test]
    fn start_returns_ok_on_empty_registry() {
        let mut svc = AgentService::new(test_ctx(InstanceId(7)));
        assert!(svc.start().is_ok(), "空骨架 start 必须 Ok");
        assert!(svc.session_view().sessions.is_empty(), "无会话");
    }

    #[test]
    fn factory_yields_startable_service() {
        let mut boxed: Box<dyn Service> = agent_service_factory(test_ctx(InstanceId(9)));
        assert!(boxed.start().is_ok(), "工厂出厂即可 start");
    }

    /// 构造 tell 形态信封（无 corr — 输入/尺寸/主题均 D6 静默）。
    fn tell_env(kind: &'static str, payload: Payload) -> Envelope {
        Envelope::new(
            Route::Instance(InstanceId(1)),
            Address::instance(InstanceId(99)),
            kind,
            payload,
        )
    }

    /// task 2.2 验证: 成功三支回执 + active 退位 + 未知/重复 id 幂等 Err。
    #[test]
    fn control_select_create_close_replies_and_fallback() {
        let script = Arc::new(Mutex::new(StubScript::default()));
        let msg = Arc::new(TestMessaging::new(InstanceId(5)));
        let mut svc = AgentService::with_factory(
            test_ctx_with(Arc::clone(&msg)),
            stub_factory(Arc::clone(&script)),
        );
        let a = svc.insert_session(
            "pi-1".into(),
            PathBuf::from("/p1"),
            Box::new(StubSession::new(Arc::clone(&script))),
        );
        let b = svc.insert_session(
            "pi-1".into(),
            PathBuf::from("/p2"),
            Box::new(StubSession::new(Arc::clone(&script))),
        );

        // select 成功 → active 归属
        svc.dispatch(ctrl_env(KIND_SELECT, Payload::new(SelectReq(a)), 1));
        assert_eq!(svc.active(), Some(a));
        assert_eq!(last_reply::<SelectOk>(&msg, 1), Ok(SelectOk));

        // select 未知 id → Err 回执 (状态不变)
        svc.dispatch(ctrl_env(
            KIND_SELECT,
            Payload::new(SelectReq(SessionId(999))),
            2,
        ));
        assert!(matches!(
            last_reply::<SelectOk>(&msg, 2),
            Err(e) if e.0.contains("no such")
        ));
        assert_eq!(svc.active(), Some(a), "失败 select 不动 active");

        // create 成功 → 新 id + 置 active + 入清单
        svc.dispatch(ctrl_env(
            KIND_CREATE,
            Payload::new(CreateReq {
                root: PathBuf::from("/p3"),
            }),
            3,
        ));
        let created = last_reply::<CreateOk>(&msg, 3).expect("create ok").0;
        assert_eq!(svc.active(), Some(created), "新建即 active");
        assert_eq!(svc.session_view().sessions.len(), 3, "新行入清单");
        assert_eq!(
            svc.session_view().sessions[2].name, "p3/pi-1",
            "/p3 首个名额 `<project>/pi-1` (spec)"
        );

        // close 非 active → 不动 active; io 必须 shutdown
        svc.dispatch(ctrl_env(KIND_CLOSE, Payload::new(CloseReq(a)), 4));
        assert_eq!(last_reply::<CloseOk>(&msg, 4), Ok(CloseOk));
        assert!(script.lock().expect("script").shutdowns >= 1, "close 须 shutdown io");
        assert_eq!(svc.active(), Some(created));

        // close active → 退位剩余最后一个 (b)
        svc.dispatch(ctrl_env(KIND_CLOSE, Payload::new(CloseReq(created)), 5));
        assert_eq!(last_reply::<CloseOk>(&msg, 5), Ok(CloseOk));
        assert_eq!(svc.active(), Some(b), "退位剩余最后一个");

        // close b → 无剩余 → None
        svc.dispatch(ctrl_env(KIND_CLOSE, Payload::new(CloseReq(b)), 6));
        assert_eq!(last_reply::<CloseOk>(&msg, 6), Ok(CloseOk));
        assert_eq!(svc.active(), None, "无剩余 → None");

        // 重复 close → 幂等 Err
        svc.dispatch(ctrl_env(KIND_CLOSE, Payload::new(CloseReq(b)), 7));
        assert!(matches!(
            last_reply::<CloseOk>(&msg, 7),
            Err(e) if e.0.contains("no such")
        ));

        // 全部关完再建同 root → p3/pi-2: 名额计数不随关闭回退
        // (N 每项目起 1, 同项目内不复用 — spec)
        svc.dispatch(ctrl_env(
            KIND_CREATE,
            Payload::new(CreateReq {
                root: PathBuf::from("/p3"),
            }),
            8,
        ));
        last_reply::<CreateOk>(&msg, 8).expect("second create ok");
        let view = svc.session_view();
        assert_eq!(view.sessions.len(), 1, "只剩新会话");
        assert_eq!(view.sessions[0].name, "p3/pi-2", "名额递增不复用 (spec)");
    }

    /// task 2.2 验证: create 失败 → Err(String)、无新行、active 不动。
    #[test]
    fn create_failure_returns_err_without_session() {
        let msg = Arc::new(TestMessaging::new(InstanceId(6)));
        let failing: crate::session::SessionFactory =
            Arc::new(|_, _, _| Err("spawn failed: pi missing".into()));
        let mut svc = AgentService::with_factory(test_ctx_with(Arc::clone(&msg)), failing);

        svc.dispatch(ctrl_env(
            KIND_CREATE,
            Payload::new(CreateReq {
                root: PathBuf::from("/p"),
            }),
            1,
        ));
        assert!(matches!(
            last_reply::<CreateOk>(&msg, 1),
            Err(e) if e.0.contains("pi missing")
        ));
        assert!(svc.session_view().sessions.is_empty(), "失败无新行");
        assert_eq!(svc.active(), None, "失败不动 active");
    }

    /// task 2.3 验证: 输入四类转发 + resize 服务侧去重 + theme 广播与
    /// create 快照注入；tell 全程无出站（D6 静默）。
    #[test]
    fn input_resize_theme_forward_and_dedupe() {
        let script = Arc::new(Mutex::new(StubScript::default()));
        let msg = Arc::new(TestMessaging::new(InstanceId(7)));
        let mut svc = AgentService::with_factory(
            test_ctx_with(Arc::clone(&msg)),
            stub_factory(Arc::clone(&script)),
        );
        let id = svc.insert_session(
            "pi-1".into(),
            PathBuf::from("/p"),
            Box::new(StubSession::new(Arc::clone(&script))),
        );

        // 输入四类 → 桩记录
        let key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('a'),
            crossterm::event::KeyModifiers::empty(),
        );
        svc.dispatch(tell_env(
            KIND_INPUT_KEY,
            Payload::new(InputKey { session: id, key }),
        ));
        svc.dispatch(tell_env(
            KIND_INPUT_TEXT,
            Payload::new(InputText {
                session: id,
                text: "hi".into(),
            }),
        ));
        svc.dispatch(tell_env(
            KIND_INPUT_WHEEL,
            Payload::new(InputWheel { session: id, lines: -3 }),
        ));
        svc.dispatch(tell_env(
            KIND_INPUT_CLICK,
            Payload::new(InputClick { session: id, x: 4, y: 5 }),
        ));
        {
            let s = script.lock().expect("script");
            assert_eq!(s.keys.len(), 1, "key 转发");
            assert_eq!(s.texts, vec!["hi".to_string()], "text 转发");
            assert_eq!(s.wheels, vec![-3], "wheel 转发");
            assert_eq!(s.clicks, vec![(4, 5)], "click 转发");
        }
        assert!(msg.sent().is_empty(), "tell 静默: 无出站信封 (D6)");

        // resize 去重: 同尺寸不二次转发, 变尺寸才转
        svc.dispatch(tell_env(
            KIND_RESIZE,
            Payload::new(ResizeReq {
                session: id,
                cols: 80,
                rows: 24,
            }),
        ));
        svc.dispatch(tell_env(
            KIND_RESIZE,
            Payload::new(ResizeReq {
                session: id,
                cols: 80,
                rows: 24,
            }),
        ));
        assert_eq!(
            script.lock().expect("script").resized,
            vec![(80, 24)],
            "同尺寸去重"
        );
        svc.dispatch(tell_env(
            KIND_RESIZE,
            Payload::new(ResizeReq {
                session: id,
                cols: 100,
                rows: 30,
            }),
        ));
        assert_eq!(
            script.lock().expect("script").resized,
            vec![(80, 24), (100, 30)],
            "变尺寸转发"
        );

        // theme: 广播既有会话 + 快照供后续 create 注入
        let told = TuiTheme {
            syntax_theme: "base16-eighties.dark".into(),
            ..Default::default()
        };
        svc.dispatch(tell_env(
            KIND_THEME,
            Payload::new(ThemeReq {
                theme: told.clone(),
            }),
        ));
        {
            let s = script.lock().expect("script");
            assert_eq!(s.themes, vec![told.clone()], "广播到既有会话");
        }
        assert_eq!(svc.theme, told, "服务侧快照更新");

        // 新 create 注入最近快照 (非默认值 → 证明确实是快照而非写死默认)
        svc.dispatch(tell_env(
            KIND_CREATE,
            Payload::new(CreateReq {
                root: PathBuf::from("/p2"),
            }),
        ));
        assert_eq!(
            script.lock().expect("script").create_themes,
            vec![told],
            "create 注入服务持有的最近 theme 快照"
        );
    }

    /// task 2.5 验证: select/create/close/status 变化 → 全量快照投影且
    /// active 正确；无变化零事件（同 id select / idle tick）。
    #[test]
    fn projection_events_carry_full_snapshot_and_active() {
        use crate::contract::{KIND_SESSIONS_CHANGED, SessionsChanged};

        let script = Arc::new(Mutex::new(StubScript::default()));
        let msg = Arc::new(TestMessaging::new(InstanceId(9)));
        let mut svc = AgentService::with_factory(
            test_ctx_with(Arc::clone(&msg)),
            stub_factory(Arc::clone(&script)),
        );
        let a = svc.insert_session(
            "pi-1".into(),
            PathBuf::from("/p1"),
            Box::new(StubSession::new(Arc::clone(&script))),
        );

        let changes = |msg: &TestMessaging| -> Vec<Envelope> {
            msg.sent()
                .into_iter()
                .filter(|e| e.kind == KIND_SESSIONS_CHANGED)
                .collect()
        };
        let last_view = |evs: &[Envelope]| -> SessionView {
            evs.last()
                .expect("必须有投影事件")
                .downcast::<SessionsChanged>()
                .expect("SessionsChanged 载荷")
                .0
                .clone()
        };

        // watch → 冷启动全量快照 (1 条, active None)
        svc.dispatch(Envelope::new(
            Route::Instance(InstanceId(1)),
            Address::instance(InstanceId(77)),
            KIND_WATCH,
            Payload::new(WatchReq),
        ));
        let evs = changes(&msg);
        assert_eq!(evs.len(), 1, "watch 即推冷启动快照");
        let view = last_view(&evs);
        assert_eq!(view.sessions.len(), 1, "全量清单");
        assert_eq!(view.active, None, "初始无 active");

        // select → active 变化 → 新事件
        svc.dispatch(ctrl_env(KIND_SELECT, Payload::new(SelectReq(a)), 1));
        let evs = changes(&msg);
        assert_eq!(evs.len(), 2, "select 变化推投影");
        assert_eq!(last_view(&evs).active, Some(a));

        // select 同 id → 无变化 → 零新事件
        svc.dispatch(ctrl_env(KIND_SELECT, Payload::new(SelectReq(a)), 2));
        assert_eq!(changes(&msg).len(), 2, "同 id select 零事件");

        // create → 注册表 + active 变化
        svc.dispatch(ctrl_env(
            KIND_CREATE,
            Payload::new(CreateReq {
                root: PathBuf::from("/p2"),
            }),
            3,
        ));
        let evs = changes(&msg);
        assert_eq!(evs.len(), 3, "create 推投影");
        let view = last_view(&evs);
        assert_eq!(view.sessions.len(), 2, "快照含新会话");
        let created = view.active.expect("create 置 active");
        assert_eq!(Some(created), svc.active());

        // close 非 active → 注册表变化
        svc.dispatch(ctrl_env(KIND_CLOSE, Payload::new(CloseReq(a)), 4));
        let evs = changes(&msg);
        assert_eq!(evs.len(), 4, "close 推投影");
        let view = last_view(&evs);
        assert_eq!(view.sessions.len(), 1, "快照移除已关");
        assert_eq!(view.active, Some(created), "非 active 关闭不动 active");

        // status 变化 → 快照携带新状态
        script.lock().expect("script").pending_bytes = b"out".to_vec();
        svc.tick();
        let evs = changes(&msg);
        assert_eq!(evs.len(), 5, "状态翻转推投影");
        assert_eq!(
            last_view(&evs).sessions[0].status,
            SessionStatus::Running,
            "快照携带 Running"
        );

        // idle tick → 零事件
        svc.tick();
        assert_eq!(changes(&msg).len(), 5, "idle 零投影");
    }

    /// task 2.6a 验证: 输入内联 drain — 发 input 后立即成帧, 无需手动 tick。
    #[test]
    fn input_triggers_inline_drain() {
        let script = Arc::new(Mutex::new(StubScript::default()));
        let msg = Arc::new(TestMessaging::new(InstanceId(10)));
        let mut svc = AgentService::with_factory(
            test_ctx_with(Arc::clone(&msg)),
            stub_factory(Arc::clone(&script)),
        );
        let a = svc.insert_session(
            "pi-1".into(),
            PathBuf::from("/p"),
            Box::new(StubSession::new(Arc::clone(&script))),
        );
        svc.dispatch(Envelope::new(
            Route::Instance(InstanceId(1)),
            Address::instance(InstanceId(77)),
            KIND_WATCH,
            Payload::new(WatchReq),
        ));

        script.lock().expect("script").pending_bytes = b"burst".to_vec();
        svc.dispatch(tell_env(
            KIND_INPUT_TEXT,
            Payload::new(InputText {
                session: a,
                text: "x".into(),
            }),
        ));

        let frames: Vec<Envelope> = msg
            .sent()
            .into_iter()
            .filter(|e| e.kind == KIND_FRAME_BYTES)
            .collect();
        assert_eq!(frames.len(), 1, "输入即推帧 (内联 drain, 未手动 tick)");
        let fb = frames[0].downcast::<FrameBytes>().expect("FrameBytes");
        assert_eq!(fb.bytes.as_slice(), b"burst", "内联 drain 取到新字节");
        assert_eq!(
            script.lock().expect("script").texts,
            vec!["x".to_string()],
            "输入已先转发"
        );
    }

    /// task 2.6b 验证: drain 循环真实上宿主 runtime 按拍自寻址；
    /// 回注一拍（投递职责归 manager, 此处直调）→ tick → 成帧。
    #[test]
    fn drain_loop_emits_ticks_on_host_runtime() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("test runtime");
        let script = Arc::new(Mutex::new(StubScript::default()));
        let msg = Arc::new(TestMessaging::new(InstanceId(11)));
        msg.set_runtime(rt.handle().clone());
        let mut svc = AgentService::with_factory(
            test_ctx_with(Arc::clone(&msg)),
            stub_factory(Arc::clone(&script)),
        );
        let a = svc.insert_session(
            "pi-1".into(),
            PathBuf::from("/p"),
            Box::new(StubSession::new(Arc::clone(&script))),
        );
        svc.dispatch(Envelope::new(
            Route::Instance(InstanceId(1)),
            Address::instance(InstanceId(77)),
            KIND_WATCH,
            Payload::new(WatchReq),
        ));
        script.lock().expect("script").pending_bytes = b"loop".to_vec();

        assert!(svc.start().is_ok(), "start 起 drain 循环");
        // 驱动 runtime 130ms: 首拍(立即)已跳过, 50/100ms 两拍应已发
        rt.block_on(async { tokio::time::sleep(Duration::from_millis(130)).await });

        let ticks: Vec<Envelope> = msg
            .sent()
            .into_iter()
            .filter(|e| e.kind == KIND_DRAIN_TICK)
            .collect();
        assert!(!ticks.is_empty(), "循环在 50ms 拍点自寻址 (130ms ≥1 拍)");

        let env = ticks[0].clone();
        let _fut = svc.handle(env); // handle 内同步 dispatch → tick
        let frames: Vec<Envelope> = msg
            .sent()
            .into_iter()
            .filter(|e| e.kind == KIND_FRAME_BYTES)
            .collect();
        assert_eq!(frames.len(), 1, "drain.tick 驱动 tick → 成帧");
        let fb = frames[0].downcast::<FrameBytes>().expect("FrameBytes");
        assert_eq!(fb.session, a, "帧归属活跃会话");
        assert_eq!(fb.bytes.as_slice(), b"loop");
    }

    /// task 2.4 验证: watch 前零推 / 变字节推到 watcher / 无字节不推 /
    /// 全推（两会话各自成帧）/ 出口满丢帧计数且 drain 不阻塞。
    #[test]
    fn watch_receives_frame_pushes_and_drop_counts() {
        // 每会话独立剧本: 共享剧本会被先 drain 的会话抢走字节
        let script_a = Arc::new(Mutex::new(StubScript::default()));
        let script_b = Arc::new(Mutex::new(StubScript::default()));
        let msg = Arc::new(TestMessaging::new(InstanceId(8)));
        let mut svc = AgentService::with_factory(
            test_ctx_with(Arc::clone(&msg)),
            stub_factory(Arc::clone(&script_a)),
        );
        let a = svc.insert_session(
            "pi-1".into(),
            PathBuf::from("/p1"),
            Box::new(StubSession::new(Arc::clone(&script_a))),
        );
        let b = svc.insert_session(
            "pi-1".into(),
            PathBuf::from("/p2"),
            Box::new(StubSession::new(Arc::clone(&script_b))),
        );

        // watch 前: 有字节也不推 (零出站)
        script_a.lock().expect("script").pending_bytes = b"early".to_vec();
        let out = svc.tick();
        assert_eq!(out.frames.len(), 1, "drain 照常产出");
        assert!(msg.sent().is_empty(), "watch 前零推");

        // watch 握手: 订阅身份取 from (InstanceId 77)
        svc.dispatch(Envelope::new(
            Route::Instance(InstanceId(1)),
            Address::instance(InstanceId(77)),
            KIND_WATCH,
            Payload::new(WatchReq),
        ));

        // 变字节 → 一帧投到 watcher; 无字节的会话不产帧
        script_b.lock().expect("script").pending_bytes = b"from-b".to_vec();
        svc.tick();
        let frames: Vec<Envelope> = msg
            .sent()
            .into_iter()
            .filter(|e| e.kind == KIND_FRAME_BYTES)
            .collect();
        assert_eq!(frames.len(), 1, "仅 b 有字节 → 一帧");
        assert_eq!(
            frames[0].to,
            Route::Instance(InstanceId(77)),
            "帧投到 watch 的 from 地址"
        );
        let fb = frames[0].downcast::<FrameBytes>().expect("FrameBytes 载荷");
        assert_eq!(fb.session, b, "帧归属 b");
        assert_eq!(fb.bytes.as_slice(), b"from-b", "字节零拷贝透传");

        // 无字节 → 不推
        let before = msg.sent().len();
        svc.tick();
        assert_eq!(msg.sent().len(), before, "idle 不推帧");

        // 全推: 两会话同刻有字节 → 各自成帧
        script_a.lock().expect("script").pending_bytes = b"A".to_vec();
        script_b.lock().expect("script").pending_bytes = b"B".to_vec();
        svc.tick();
        let frames: Vec<Envelope> = msg
            .sent()
            .into_iter()
            .filter(|e| e.kind == KIND_FRAME_BYTES)
            .collect();
        assert_eq!(frames.len(), 3, "累计: 1 + 2 全推");
        let sessions: std::collections::HashSet<SessionId> = frames
            .iter()
            .map(|e| e.downcast::<FrameBytes>().expect("FrameBytes").session)
            .collect();
        assert_eq!(
            sessions,
            [a, b].into_iter().collect(),
            "全推覆盖全部会话"
        );

        // 出口满 → 丢帧计数, drain 不阻塞 (帧自含状态, 后帧自愈)
        msg.set_fail_tells(true);
        script_a.lock().expect("script").pending_bytes = b"drop".to_vec();
        let out = svc.tick();
        assert_eq!(out.frames.len(), 1, "drain 照常, 不因出口失败阻塞");
        assert_eq!(svc.dropped_frames(), 1, "丢帧计数递增");
    }

    /// task 2.1 验证: Starting → Running (首帧字节) → Exited (io 终止)，
    /// 且 tick 产出帧字节、无字节时零帧零变化。
    #[test]
    fn status_transitions_from_bytes_and_io_end() {
        let script = Arc::new(Mutex::new(StubScript::default()));
        let mut svc = AgentService::with_factory(
            test_ctx(InstanceId(3)),
            stub_factory(Arc::clone(&script)),
        );
        let id = svc.insert_session(
            "pi-1".into(),
            PathBuf::from("/tmp/proj"),
            Box::new(StubSession::new(Arc::clone(&script))),
        );
        assert_eq!(svc.session_status(id), Some(SessionStatus::Starting), "起始态");

        // 首批字节 → Running, 字节作为帧产出
        script.lock().expect("script").pending_bytes = b"hello".to_vec();
        let out = svc.tick();
        assert!(out.changed, "首帧必须翻转状态");
        assert_eq!(svc.session_status(id), Some(SessionStatus::Running));
        assert_eq!(
            out.frames,
            vec![(id, Arc::new(b"hello".to_vec()))],
            "帧字节所有权移动 (Arc 零拷贝)"
        );

        // 无新字节 → 零变化零帧
        let out = svc.tick();
        assert!(!out.changed && out.frames.is_empty(), "idle tick 零产出");

        // io 终止 → Exited
        script.lock().expect("script").alive = false;
        assert!(svc.tick().changed, "io 死亡必须翻转");
        assert_eq!(svc.session_status(id), Some(SessionStatus::Exited));

        // 已 Exited → 稳定
        assert!(!svc.tick().changed, "Exited 稳态不再翻转");
    }

    /// design D5 回改: 输入后 8/16/24ms 三拍补采自寻址 (40ms 窗口内
    /// 恰 3 拍 — 主 tick 首拍被跳过立即拍后落在 ~50ms, 不入窗)。
    #[test]
    fn input_spawns_followup_burst_ticks() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("test runtime");
        let script = Arc::new(Mutex::new(StubScript::default()));
        let msg = Arc::new(TestMessaging::new(InstanceId(12)));
        msg.set_runtime(rt.handle().clone());
        let mut svc = AgentService::with_factory(
            test_ctx_with(Arc::clone(&msg)),
            stub_factory(Arc::clone(&script)),
        );
        let id = svc.insert_session(
            "pi-1".into(),
            PathBuf::from("/p"),
            Box::new(StubSession::new(Arc::clone(&script))),
        );
        assert!(svc.start().is_ok());

        svc.dispatch(tell_env(
            KIND_INPUT_TEXT,
            Payload::new(InputText {
                session: id,
                text: "x".into(),
            }),
        ));
        // 真实时钟推进: 补采在 8/16/24ms 发, 主 tick 首拍 >40ms
        rt.block_on(async { tokio::time::sleep(Duration::from_millis(40)).await });
        let ticks = msg
            .sent()
            .iter()
            .filter(|e| e.kind == KIND_DRAIN_TICK)
            .count();
        assert_eq!(ticks, AgentService::INPUT_BURST_STEPS, "输入后三拍补采");
    }

    #[test]
    fn session_view_carries_full_snapshot_and_active() {
        let script = Arc::new(Mutex::new(StubScript::default()));
        let mut svc = AgentService::with_factory(
            test_ctx(InstanceId(4)),
            stub_factory(Arc::clone(&script)),
        );
        let a = svc.insert_session(
            "pi-1".into(),
            PathBuf::from("/p1"),
            Box::new(StubSession::new(Arc::clone(&script))),
        );
        let b = svc.insert_session(
            "pi-1".into(),
            PathBuf::from("/p2"),
            Box::new(StubSession::new(Arc::clone(&script))),
        );
        svc.active = Some(b);
        let view = svc.session_view();
        assert_eq!(view.sessions.len(), 2, "全量清单");
        assert_eq!(view.active, Some(b), "active 随快照携带");
        assert_eq!(view.sessions[0].name, "pi-1");
        assert_eq!(view.sessions[1].project_root, PathBuf::from("/p2"));
        let _ = a;
    }
}
