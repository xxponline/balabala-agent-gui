//! GUI 侧服务接入（service-management-framework task 6.1）。
//!
//! **GUI 就是总线上的拉模式参与者**（design D9/D10）：
//! - 启动时 bootstrap：扫描 `plugins/` 建 Catalog、装载核心假服务、
//!   注册 GUI 自己的 mailbox（`Listener`）、订阅状态翻转（订阅 = 路由规则）；
//! - 泵：`pump()` 并入现有 50ms drain 循环（`poll_sessions`）——listener
//!   poll 同步、GUI 主线程执行，**无回调、无跨线程合同**；
//! - 用户触发的操作一律走 ask（`ask_core`）——错误可回显；tell 只用于
//!   fire-and-forget（D6 约定）。
//!
//! 不含服务状态页 UX（6.2 验收边界：留给后续 GUI spec change），不迁移
//! 任何现有模块。

use agent_service::contract::{
    AgentContract, CloseOk, ControlResult, CreateOk, FrameBytes, SelectOk, SessionView,
    SessionsChanged, WatchReq, KIND_CONTROL_REPLY, KIND_FRAME_BYTES, KIND_SESSIONS_CHANGED,
    KIND_WATCH,
};
use service_manager::{
    ContractId, Corr, Envelope, InstanceId, Listener, Payload, Route, Service, ServiceCtx,
    ServiceFuture, ServiceManager, ServiceState, StateChange, KIND_ROUTE_ERROR, KIND_STATE_CHANGE,
};
// change solution-service task 2.2: Solution/Project 域服务接入（design D1）。
use solution_service::contract::{
    OpError as SolOpError, OpResult as SolOpResult, SolutionContract, TreeChanged as SolTreeChanged,
    TreeSnapshot, WatchReq as SolWatchReq, KIND_LOAD, KIND_OP_REPLY as KIND_SOLUTION_REPLY,
    KIND_TREE_CHANGED as KIND_SOLUTION_CHANGED, KIND_WATCH as KIND_SOLUTION_WATCH,
};
use solution_service::service::solution_service_factory;
use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::Path;

/// 控制操作类别（回执 downcast 分派；task 3.1）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(dead_code)] // Submit/Create/Close 由 task 3.2 app 控制链构造
pub enum CtrlOp {
    Select,
    Create,
    Close,
}

/// GUI → core.fake 的请求（**归调用方**，D2 归属规则演示）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CorePing(pub u64);

/// core.fake 的应答。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CorePong(pub u64);

/// 核心假服务（framework 自带的最小可服务载体；真模块迁移属后续 change）。
struct CoreFake {
    ctx: ServiceCtx,
}

impl Service for CoreFake {
    fn start(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn handle(&mut self, env: service_manager::Envelope) -> ServiceFuture<'_> {
        if env.corr.is_none() {
            // tell 语义: 无回复; 未知消息拒收落死信 (spec R1)
            let _ = self.ctx.messaging.reject(&env);
            return Box::pin(std::future::ready(()));
        }
        match env.downcast::<CorePing>() {
            Ok(ping) => {
                let _ = self
                    .ctx
                    .messaging
                    .reply(&env, "core.pong", Payload::new(CorePong(ping.0)));
            }
            Err(_) => {
                let _ = self.ctx.messaging.reject(&env);
            }
        }
        Box::pin(std::future::ready(()))
    }
}

fn core_fake_factory(ctx: ServiceCtx) -> Box<dyn Service> {
    Box::new(CoreFake { ctx })
}

/// 回执 `ControlResult<T>` 拆错（Ok → None；Err/类型不匹配 → Some 文案）。
fn failed_of<T: Any + Send + Sync>(env: &Envelope) -> Option<String> {
    match env.downcast::<ControlResult<T>>() {
        Ok(Ok(_)) => None,
        Ok(Err(e)) => Some(e.0.clone()),
        Err(_) => Some("回执类型不匹配".to_string()),
    }
}

/// 生产会话工厂: 真实嵌入 (pi --mode rpc 子进程; task 3.2)。
/// 测试构建不编译本函数 — 测试工厂由 task 3.3 接入 (cfg 选择见 bootstrap)。
#[cfg(not(test))]
fn spawn_pi_session(
    name: String,
    root: std::path::PathBuf,
    theme: pi_shell_rs::app::theme::TuiTheme,
) -> Result<Box<dyn agent_service::session::SessionIo>, String> {
    Ok(Box::new(agent_service::PiAgentShell::new(
        name, root, theme,
    )?))
}

/// 生产工厂: `AgentService` + 真实 spawn (task 3.2, design D2)。
#[cfg(not(test))]
fn gui_agent_factory(ctx: ServiceCtx) -> Box<dyn Service> {
    Box::new(agent_service::AgentService::with_factory(
        ctx,
        std::sync::Arc::new(spawn_pi_session),
    ))
}

#[cfg(test)]
pub(crate) mod test_sessions;

/// 测试工厂: 可检视测试会话 (登记表按 root 隔离; task 3.2/3.3)。
#[cfg(test)]
fn gui_test_agent_factory(ctx: ServiceCtx) -> Box<dyn Service> {
    Box::new(agent_service::AgentService::with_factory(
        ctx,
        test_sessions::factory(),
    ))
}

/// GUI 参与者的服务层状态（订阅 + 本地快照，D10 投影所有权在 GUI 侧）。
pub struct ServiceLayer {
    /// 总线门面（宿主 API，零 tokio/kameo 类型，D9）。
    pub manager: ServiceManager,
    /// GUI 自己的 mailbox（拉模式 actor 句柄）。
    pub listener: Listener,
    /// 核心假服务实例 id。
    pub core_id: InstanceId,
    /// agent 会话薄服务实例 id（change agent-service task 1.3；控制/帧路由目标）。
    #[allow(dead_code)] // task 3.1 watch/控制路由接线后读取
    pub agent_id: InstanceId,
    /// 经泵消费到的事件数（测试观测面：证明 Listener 接入 gpui 泵）。
    pub events_seen: usize,
    /// 各实例最新状态（D10 本地快照按实例口径；状态页 UX 属后续 change）。
    /// 单槽"最近翻转"会被后到的无关实例事件踩掉（实测: agent 启动事件迟到
    /// 覆盖 core 的 Offline）— 断言/消费方一律按实例查表。
    pub states: HashMap<InstanceId, ServiceState>,
    /// `sessions_changed` 投影镜像（task 3.1；sidebar/active 渲染源）。
    pub sessions_view: Option<SessionView>,
    /// 待消费帧（task 3.1；app 泵转发给 AgentTerminal.feed）。
    frames: Vec<FrameBytes>,
    /// 控制失败 notice 队列（spec delta: 点名失败操作）。
    notices: Vec<String>,
    /// 已发控制台账: corr -> (操作, 标签)（回执/route_error 配对）。
    pending_ops: HashMap<Corr, (CtrlOp, &'static str)>,
    /// 控制成功回执（task 3.2: app 据此切 console tab — 成功才切，spec delta）。
    successes: Vec<CtrlOp>,
    /// solution 操作成功回执标签（非 load 的 Ok 配对；表单签自关消费 —
    /// solution-config-tab 3.1）。
    sol_successes: Vec<&'static str>,
    /// watch 握手已发出（bootstrap 时 agent 尚 loading 会被 fail-fast 拒 →
    /// 首个 Available 事件到达时补发，task 3.1）。
    watch_sent: bool,
    /// agent 就绪一次性信号 (pump 见 Available 置位; app 据此补推主题 —
    /// 服务快照默认深色, 宿主主题为准 — task 3.2)。
    agent_ready: bool,
    /// solution 域服务实例 id（change solution-service task 2.2；树路由目标）。
    pub solution_id: InstanceId,
    /// 项目树镜像（`tree_changed` 快照 + 操作回执自带快照；视图/交树渲染源）。
    tree_view: Option<TreeSnapshot>,
    /// 测试注入覆盖 (优先于真实镜像 — 无头测试不经真实 load 时的视图
    /// 数据源; 服务首推空快照不得踩掉注入值)。
    #[cfg(test)]
    tree_view_override: Option<TreeSnapshot>,
    /// solution watch 握手已发出（同 agent 的 loading 规避时序，task 2.2）。
    solution_watch_sent: bool,
    /// solution 操作台账: corr -> 标签（回执/route_error 配对）。
    pending_sol_ops: HashMap<Corr, &'static str>,
    /// solution load 台账: corr 集合（回执进 `load_results` 供打开链路拉取）。
    pending_loads: HashSet<Corr>,
    /// load 回执队列: corr -> 回执（打开链路轮询消费——pull 模式纪律：
    /// 不 oneshot await，确定性测试调度器禁止外线程唤醒，design D8）。
    load_results: Vec<(Corr, SolOpResult)>,
}

/// core.fake 契约类标记（change `singleton-per-contract`, design D1）：
/// 内部核心假服务的骨架类身份（零大小 marker，只承载 `ContractId`）。
struct CoreFakeContract;

impl ServiceLayer {
    /// 启动接线：扫描 `scan_dir`（= 设计中的 `plugins/` 落点）建 Catalog，
    /// 注册并装载核心假服务，注册 GUI mailbox、订阅其状态翻转。
    pub fn bootstrap(scan_dir: &Path) -> Result<Self, String> {
        let manager = ServiceManager::new(scan_dir).map_err(|e| format!("service layer: {e}"))?;
        // 契约登记（change `singleton-per-contract`, design D2）：宿主契约表供
        // DLL 装载期按契约符号解析（demo = 示例插件）；agent 为内部单例类
        // 身份，自述登记（内部装载不经此表，登记保持契约名表完整）。
        manager.register_contract("demo", ContractId::of::<contracts::demo::DemoContract>());
        manager.register_contract("agent", ContractId::of::<AgentContract>());
        manager
            .register_internal(
                "core.fake",
                core_fake_factory,
                ContractId::of::<CoreFakeContract>(),
            )
            .map_err(|e| format!("register core.fake: {e}"))?;
        let core_id = manager
            .load(&"core.fake".into())
            .map_err(|e| format!("load core.fake: {e}"))?;
        // change agent-service task 1.3: 进程内会话薄服务 (design D1)。
        // 工厂按编译目标选择: 生产 = 真 spawn (PiAgentShell); 测试构建 =
        // 可检视测试会话 (登记表按 root 隔离 — 并行测试零互扰)。
        #[cfg(not(test))]
        let factory: service_manager::ServiceFactory = gui_agent_factory;
        #[cfg(test)]
        let factory: service_manager::ServiceFactory = gui_test_agent_factory;
        manager
            .register_internal("agent", factory, ContractId::of::<AgentContract>())
            .map_err(|e| format!("register agent: {e}"))?;
        let agent_id = manager
            .load(&"agent".into())
            .map_err(|e| format!("load agent: {e}"))?;
        // change solution-service task 2.2: Solution/Project 域服务 (design D1)。
        manager.register_contract("solution", ContractId::of::<SolutionContract>());
        manager
            .register_internal(
                "solution",
                solution_service_factory,
                ContractId::of::<SolutionContract>(),
            )
            .map_err(|e| format!("register solution: {e}"))?;
        let solution_id = manager
            .load(&"solution".into())
            .map_err(|e| format!("load solution: {e}"))?;
        // GUI = 普通参与者: 自己的 mailbox + 订阅 (= 往路由表加规则, D7/D9)
        // watch 握手不在这里发: 刚 load 的实例仍 loading, tell 会被 fail-fast
        // 拒 (target not available) — 由 pump 见 agent Available 事件时补发。
        let listener = manager.new_mailbox(64);
        manager.subscribe_state(core_id, &listener);
        manager.subscribe_state(agent_id, &listener);
        manager.subscribe_state(solution_id, &listener);
        // 订阅时序补给 (竞态修复): driver 的 InitOk 翻转是异步任务, 可能抢在
        // subscribe_state 之前发布 — 订阅表还空, 启动状态事件被静默丢弃
        // (实测: 高负载并行下 watch 握手因此断链, 会话投影永不产生)。
        // 订阅后按快照补发等价事件: 与真事件同形 (from=Loading), 消费方幂等
        // (watch/agent_ready 均一次性守卫), 重复到达无害。
        for (id, name) in [
            (core_id, "core.fake"),
            (agent_id, "agent"),
            (solution_id, "solution"),
        ] {
            let Some(state) = manager.supervisor().state_of(id) else {
                continue; // 未装载: 不可能 (上文刚 load)
            };
            if state == ServiceState::Loading {
                continue; // 尚未翻转: 真事件晚些必达 (订阅已在位)
            }
            let env = Envelope::new(
                Route::Instance(listener.id()),
                listener.address().clone(),
                KIND_STATE_CHANGE,
                Payload::new(StateChange {
                    instance: id,
                    service: name.to_string().into(),
                    from: ServiceState::Loading,
                    to: state,
                }),
            );
            let _ = manager.tell(env);
        }
        Ok(ServiceLayer {
            manager,
            listener,
            core_id,
            agent_id,
            events_seen: 0,
            states: HashMap::new(),
            sessions_view: None,
            frames: Vec::new(),
            notices: Vec::new(),
            pending_ops: HashMap::new(),
            successes: Vec::new(),
            sol_successes: Vec::new(),
            watch_sent: false,
            agent_ready: false,
            solution_id,
            tree_view: None,
            #[cfg(test)]
            tree_view_override: None,
            solution_watch_sent: false,
            pending_sol_ops: HashMap::new(),
            pending_loads: HashSet::new(),
            load_results: Vec::new(),
        })
    }

    /// 一次消息循环迭代：poll mailbox、逐条消费（状态事件更新本地快照，
    /// 控制回执/route_error 失败入 notice 队列，帧/投影入本地镜像）。
    /// 返回是否有新件——调用方据此 `notify`。
    pub fn pump(&mut self) -> bool {
        if !self.listener.poll() {
            return false;
        }
        let mut changed = false;
        // watch 地址先取: drain 循环持有 listener 可变借用 (循环内不能再碰)
        let watch_from = self.listener.address().clone();
        for env in self.listener.drain() {
            changed = true;
            match env.kind {
                KIND_STATE_CHANGE => {
                    if let Ok(change) = env.downcast::<StateChange>() {
                        self.states.insert(change.instance, change.to);
                        // agent 就绪 → 补发 watch 握手 (D3: from 即订阅身份)
                        if !self.watch_sent
                            && change.instance == self.agent_id
                            && change.to == ServiceState::Available
                        {
                            let watch = Envelope::new(
                                Route::Instance(self.agent_id),
                                watch_from.clone(),
                                KIND_WATCH,
                                Payload::new(WatchReq),
                            );
                            if self.manager.tell(watch).is_ok() {
                                self.watch_sent = true;
                                self.agent_ready = true; // 一次性: app 补推主题
                            }
                        }
                        // change solution-service 2.2: solution watch 握手同款时序
                        if !self.solution_watch_sent
                            && change.instance == self.solution_id
                            && change.to == ServiceState::Available
                        {
                            let watch = Envelope::new(
                                Route::Instance(self.solution_id),
                                watch_from.clone(),
                                KIND_SOLUTION_WATCH,
                                Payload::new(SolWatchReq),
                            );
                            if self.manager.tell(watch).is_ok() {
                                self.solution_watch_sent = true;
                            }
                        }
                    }
                    self.events_seen += 1;
                }
                KIND_ROUTE_ERROR => {
                    // ask 失败/超时 (带 corr, 配对台账点名 — spec delta)
                    self.events_seen += 1;
                    let msg = env
                        .downcast::<service_manager::RouteError>()
                        .map(|e| e.to_string())
                        .unwrap_or_else(|_| "route error".to_string());
                    match env.corr.and_then(|c| self.pending_ops.remove(&c)) {
                        Some((_, label)) => {
                            self.notices.push(format!("{label}失败: {msg}"));
                        }
                        None => {
                            if let Some(c) = env.corr.filter(|c| self.pending_loads.remove(c)) {
                                // load 链路: 路由失败也进回执队列 (打开流程统一收口)
                                self.load_results.push((
                                    c,
                                    Err(SolOpError::Persist(format!(
                                        "solution 服务请求失败: {msg}"
                                    ))),
                                ));
                            } else {
                                self.notices.push(format!("服务请求失败: {msg}"));
                            }
                        }
                    }
                }
                // ---- task 3.1: 分流 ----
                KIND_SESSIONS_CHANGED => {
                    self.events_seen += 1;
                    if let Ok(changed) = env.downcast::<SessionsChanged>() {
                        self.sessions_view = Some(changed.0.clone());
                    }
                }
                KIND_FRAME_BYTES => {
                    self.events_seen += 1;
                    if let Ok(frame) = env.downcast::<FrameBytes>() {
                        self.frames.push(frame.clone());
                    }
                }
                KIND_CONTROL_REPLY => {
                    self.events_seen += 1;
                    let paired = env.corr.and_then(|c| self.pending_ops.remove(&c));
                    if let Some((op, label)) = paired {
                        let failed = match op {
                            CtrlOp::Select => failed_of::<SelectOk>(&env),
                            CtrlOp::Create => failed_of::<CreateOk>(&env),
                            CtrlOp::Close => failed_of::<CloseOk>(&env),
                        };
                        match failed {
                            Some(err) => self.notices.push(format!("{label}失败: {err}")),
                            None => self.successes.push(op),
                        }
                    }
                }
                // ---- change solution-service 2.2: 树镜像 + 操作回执 ----
                KIND_SOLUTION_CHANGED => {
                    self.events_seen += 1;
                    if let Ok(changed) = env.downcast::<SolTreeChanged>() {
                        self.tree_view = Some(changed.0.clone());
                    }
                }
                KIND_SOLUTION_REPLY => {
                    self.events_seen += 1;
                    let corr = env.corr;
                    let is_load = corr.is_some_and(|c| self.pending_loads.remove(&c));
                    let paired = corr.and_then(|c| self.pending_sol_ops.remove(&c));
                    match env.downcast::<SolOpResult>() {
                        Ok(res) => {
                            let res = res.clone();
                            if let Ok(snap) = &res {
                                // 回执自带快照: 镜像零回程（对齐 SessionsChanged 纪律）
                                self.tree_view = Some(snap.clone());
                            }
                            if is_load {
                                // 打开链路: 落回执队列, 由 begin_open 的轮询拉取
                                if let Some(c) = corr {
                                    self.load_results.push((c, res));
                                }
                            } else if let Err(e) = &res {
                                if let Some(label) = paired {
                                    self.notices.push(format!("{label}失败: {e}"));
                                }
                            } else if let Some(label) = paired {
                                // 非 load 的 Ok 配对标签 → 成功队列
                                // (表单签自关, solution-config-tab 3.1)
                                self.sol_successes.push(label);
                            }
                        }
                        Err(_) => {
                            if let Some(label) = paired {
                                self.notices.push(format!("{label}失败: 回执类型不匹配"));
                            }
                        }
                    }
                }
                _ => {
                    self.events_seen += 1;
                }
            }
        }
        changed
    }

    /// 控制操作 ask 通道（task 3.1；D6 用户操作一律 ask）：发件并记台账，
    /// 回执/route_error 由 `pump` 配对 → 失败入 notice（spec delta 点名操作）。
    #[allow(dead_code)] // task 3.2 app 控制链接线
    pub fn submit_control(
        &mut self,
        route: Route,
        op: CtrlOp,
        kind: &'static str,
        payload: Payload,
        label: &'static str,
    ) -> Result<Corr, service_manager::RouteError> {
        let corr = self.listener.submit(route, kind, payload)?;
        self.pending_ops.insert(corr, (op, label));
        Ok(corr)
    }

    /// solution 操作 ask 通道（change solution-service 2.2；D6 约定同
    /// `submit_control`——用户操作一律 ask）：回执/route_error 由 `pump`
    /// 配对，失败入 notice；成功回执自带树快照进镜像。
    #[allow(dead_code)] // app 控制链在 5.x 接线
    pub fn submit_solution_op(
        &mut self,
        kind: &'static str,
        payload: Payload,
        label: &'static str,
    ) -> Result<Corr, service_manager::RouteError> {
        let corr = self
            .listener
            .submit(Route::Service("solution".into()), kind, payload)?;
        self.pending_sol_ops.insert(corr, label);
        Ok(corr)
    }

    /// 当前项目树镜像（视图/交树渲染源；`tree_changed` 快照 + 操作回执）。
    #[allow(dead_code)] // 视图读取在 5.x 接线
    pub fn tree_view(&self) -> Option<&TreeSnapshot> {
        #[cfg(test)]
        if let Some(snap) = self.tree_view_override.as_ref() {
            return Some(snap);
        }
        self.tree_view.as_ref()
    }

    /// 测试缝: 注入项目树镜像 (无头测试不经真实 load 时的视图数据源;
    /// 优先于真实镜像, 不被服务首推空快照踩掉)。
    #[cfg(test)]
    pub(crate) fn set_tree_view_for_test(&mut self, snap: TreeSnapshot) {
        self.tree_view_override = Some(snap);
    }

    /// solution load ask（打开链路；**pull 模式纪律**——回执经泵落
    /// [`Self::take_load_result`] 由调用点轮询拉取，不经 oneshot await：
    /// 确定性测试调度器禁止外线程唤醒，design D8）。
    pub fn submit_solution_load(
        &mut self,
        payload: Payload,
    ) -> Result<Corr, service_manager::RouteError> {
        let corr = self
            .listener
            .submit(Route::Service("solution".into()), KIND_LOAD, payload)?;
        self.pending_loads.insert(corr);
        Ok(corr)
    }

    /// 按 corr 取走一条 load 回执（打开链路轮询）。
    pub fn take_load_result(&mut self, corr: Corr) -> Option<SolOpResult> {
        let ix = self.load_results.iter().position(|(c, _)| *c == corr)?;
        Some(self.load_results.remove(ix).1)
    }

    /// 放弃一条 load（超时收尾：摘台账，迟到回执将被无主丢弃）。
    pub fn abandon_load(&mut self, corr: Corr) {
        self.pending_loads.remove(&corr);
    }

    /// 取走待转发帧（app 泵 → AgentTerminal.feed）。
    #[allow(dead_code)] // task 3.1/3.4 app 泵接线
    pub fn take_frames(&mut self) -> Vec<FrameBytes> {
        std::mem::take(&mut self.frames)
    }

    /// 取走失败 notice（app 消费 → `app.notice`）。
    #[allow(dead_code)] // task 3.2 app.notice 接线
    pub fn take_notices(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notices)
    }

    /// 取走控制成功回执（app 据此切 console tab；失败已入 notice）。
    pub fn take_successes(&mut self) -> Vec<CtrlOp> {
        std::mem::take(&mut self.successes)
    }

    /// 取走 solution 操作成功回执标签（表单签自关消费 —
    /// solution-config-tab 3.1）。
    pub fn take_sol_successes(&mut self) -> Vec<&'static str> {
        std::mem::take(&mut self.sol_successes)
    }

    /// 一次性: agent 首次就绪 (app 据此推主题快照)。
    pub fn take_agent_ready(&mut self) -> bool {
        std::mem::take(&mut self.agent_ready)
    }

    /// 当前会话投影镜像（sidebar/active 渲染源）。
    #[allow(dead_code)] // task 3.2 sidebar/active 接线
    pub fn sessions_view(&self) -> Option<&SessionView> {
        self.sessions_view.as_ref()
    }

    /// GUI 用户操作的 ask 通道（约定：**一律 ask，不 tell**，D6）。
    /// 返回 executor-agnostic future，gpui executor 可直接 await。
    /// （6.2 验收边界：状态页/按钮 UX 归后续 GUI spec change，本 change 只
    /// 备好 ask 通道并测通；届时用户操作从 UI 调此方法。）
    #[allow(dead_code)] // UI 接入属后续 change — 通道已测通, 非死码
    pub fn ask_core(
        &self,
        seq: u64,
    ) -> impl Future<Output = Result<CorePong, service_manager::RouteError>> + Send + 'static {
        let env = service_manager::Envelope::new(
            Route::Instance(self.core_id),
            self.listener.address().clone(),
            "core.ping",
            Payload::new(CorePing(seq)),
        );
        let fut = self.manager.call(env); // 'static,急切分发 (D9)
        async move {
            let payload = fut.await?;
            payload
                .downcast::<CorePong>()
                .copied()
                .ok_or(service_manager::RouteError::TypeMismatch {
                    expected: "service_layer::CorePong",
                })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_service::contract::SelectReq;
    use service_manager::Address;
    use std::time::{Duration, Instant};

    /// 1.2: demo 与 agent 契约 marker 类型身份互异（类判据不串味）。
    ///
    /// 功能：断言两个契约 marker 铸出的 `ContractId` 不同。
    /// 输入：无。
    /// 输出：断言通过。
    #[test]
    fn contract_markers_are_distinct() {
        assert_ne!(
            ContractId::of::<contracts::demo::DemoContract>(),
            ContractId::of::<AgentContract>()
        );
    }

    fn wait_until(mut cond: impl FnMut() -> bool, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if cond() {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn temp_scan(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "smf-gui-{tag}-{}-{:04x}",
            std::process::id(),
            (Instant::now().elapsed().as_nanos() ^ 0x5155) as u16
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn bootstrap_loads_core_fake_and_subscribes() {
        let dir = temp_scan("boot");
        let mut layer = ServiceLayer::bootstrap(&dir).expect("bootstrap");
        assert!(layer.manager.has_service(&"core.fake".into()), "Catalog 登记");
        assert_eq!(
            layer.manager.instance_count(),
            3,
            "core.fake + agent + solution 均已装载"
        );

        // init 完成 (loading → available 事件应达订阅者)
        assert!(
            wait_until(
                || {
                    layer.pump();
                    layer.states.get(&layer.core_id) == Some(&ServiceState::Available)
                },
                Duration::from_secs(3)
            ),
            "状态翻转经订阅路由到达 GUI mailbox: {:?}",
            layer.states
        );
        assert!(layer.events_seen >= 1);

        // ask (GUI 用户操作约定): typed 一问一答
        let pong = futures::executor::block_on(layer.ask_core(41)).expect("ask ok");
        assert_eq!(pong, CorePong(41));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// change agent-service task 1.3: agent 薄服务注册/装载 + 状态事件到 pump。
    #[test]
    fn agent_service_loads_and_reports_state() {
        let dir = temp_scan("agent-boot");
        let mut layer = ServiceLayer::bootstrap(&dir).expect("bootstrap");
        assert!(layer.manager.has_service(&"agent".into()), "Catalog 登记 agent");
        assert_eq!(layer.manager.instance_count(), 3, "core.fake + agent + solution");
        assert_ne!(layer.agent_id, layer.core_id, "实例 id 独立分配");

        // loading → available 翻转经订阅路由到达 GUI mailbox
        assert!(
            wait_until(
                || {
                    layer.pump();
                    layer.states.get(&layer.agent_id) == Some(&ServiceState::Available)
                },
                Duration::from_secs(3)
            ),
            "agent 状态翻转到 Available: {:?}",
            layer.states
        );
        assert!(layer.events_seen >= 1, "状态事件被 pump 消费");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// change solution-service task 2.2 验证：create/configure 后树变更
    /// 通知到达且镜像更新（含注册门控对 handoff 的收缩）。
    #[test]
    fn solution_tree_changed_reaches_mirror() {
        use solution_service::contract::{
            ConfigureReq, CreateReq, KIND_CONFIGURE, KIND_CREATE,
        };

        let dir = temp_scan("sol-mirror");
        let mut layer = ServiceLayer::bootstrap(&dir).expect("bootstrap");
        assert!(
            layer.manager.has_service(&"solution".into()),
            "Catalog 登记 solution"
        );
        assert_eq!(
            layer.manager.instance_count(),
            3,
            "core.fake + agent + solution"
        );

        // solution Available + watch 握手 → 首推全量快照进镜像
        assert!(
            wait_until(
                || {
                    layer.pump();
                    layer.tree_view.is_some()
                },
                Duration::from_secs(3)
            ),
            "watch 握手后首推快照进镜像: {:?}",
            layer.states
        );

        // create: 真实项目目录（simple item 的 path 必须存在）
        let proj = dir.join("proj");
        std::fs::create_dir_all(&proj).unwrap();
        layer
            .submit_solution_op(
                KIND_CREATE,
                Payload::new(CreateReq {
                    workspace_dir: dir.clone(),
                    path: proj.clone(),
                }),
                "创建 solution",
            )
            .expect("submit create");
        assert!(
            wait_until(
                || {
                    layer.pump();
                    matches!(layer.tree_view, Some(ref t) if t.tree.projects.len() == 1)
                },
                Duration::from_secs(3)
            ),
            "create 后树变更通知到达且镜像更新"
        );
        assert_eq!(
            layer.tree_view.as_ref().unwrap().handoff.projects.len(),
            1,
            "默认双 true: 交树含新 project"
        );

        // configure: 关注册 → handoff 收缩, 全量集保留 (spec: Asset
        // registration flag gates project handoff)
        layer
            .submit_solution_op(
                KIND_CONFIGURE,
                Payload::new(ConfigureReq {
                    workspace_dir: dir.clone(),
                    solution_path: proj,
                    register_to_asset_manager: Some(false),
                    git_enabled: None,
                }),
                "配置 solution",
            )
            .expect("submit configure");
        assert!(
            wait_until(
                || {
                    layer.pump();
                    matches!(
                        layer.tree_view,
                        Some(ref t) if t.handoff.projects.is_empty() && t.tree.projects.len() == 1
                    )
                },
                Duration::from_secs(3)
            ),
            "configure 后门控交树收缩, solution 的 project 集保留"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// change agent-service task 3.1: watch 随 bootstrap 发出（trace 留痕）+
    /// 注入服务帧经 pump 分流进 GUI 帧队列。
    #[test]
    fn watch_receives_frame_bytes() {
        use agent_service::contract::SessionId;
        use std::sync::Arc;

        let dir = temp_scan("watch-frame");
        let mut layer = ServiceLayer::bootstrap(&dir).expect("bootstrap");

        // watch 握手在 agent Available 事件后补发 (pump 内), 追踪面留痕
        assert!(
            wait_until(
                || {
                    layer.pump();
                    layer
                        .manager
                        .trace_entries()
                        .iter()
                        .any(|t| t.kind == KIND_WATCH)
                },
                Duration::from_secs(3)
            ),
            "agent 就绪后必须发出 watch 握手"
        );

        // 注入服务侧帧 (服务推帧逻辑 2.4 已测; 此处测 GUI 收口分发)
        layer
            .manager
            .tell(Envelope::new(
                Route::Instance(layer.listener.id()),
                Address::instance(layer.agent_id),
                KIND_FRAME_BYTES,
                Payload::new(FrameBytes {
                    session: SessionId(1),
                    bytes: Arc::new(b"frame".to_vec()),
                    prod_gen: 0,
                }),
            ))
            .expect("inject frame");
        assert!(
            wait_until(
                || {
                    layer.pump();
                    !layer.frames.is_empty()
                },
                Duration::from_secs(3)
            ),
            "帧经 pump 分流入 GUI 队列"
        );
        let frames = layer.take_frames();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].bytes.as_slice(), b"frame", "帧字节零拷贝透传");
        assert_eq!(frames[0].session, SessionId(1));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// change agent-service task 3.1 (spec delta 场景 1 的 reply 路径):
    /// 控制回执 Err → notice 点名操作 + 台账清空无重复。
    #[test]
    fn failed_control_reply_shows_notice() {
        use agent_service::contract::SessionId;

        let dir = temp_scan("ctrl-notice");
        let mut layer = ServiceLayer::bootstrap(&dir).expect("bootstrap");

        // select 未知 id: 服务 available 前 submit 会 fail-fast (重试到可投)
        let mut submitted = false;
        assert!(
            wait_until(
                || {
                    layer.pump();
                    if !submitted {
                        let r = layer.submit_control(
                            Route::Instance(layer.agent_id),
                            CtrlOp::Select,
                            agent_service::contract::KIND_SELECT,
                            Payload::new(SelectReq(SessionId(999))),
                            "切换会话",
                        );
                        submitted = r.is_ok();
                    }
                    submitted && {
                        layer.pump();
                        !layer.notices.is_empty()
                    }
                },
                Duration::from_secs(3)
            ),
            "失败回执必须产生 notice"
        );

        let notices = layer.take_notices();
        assert!(
            notices[0].contains("切换会话"),
            "notice 点名操作: {notices:?}"
        );
        assert!(
            notices[0].contains("no such session"),
            "携带服务错误文案: {notices:?}"
        );

        // 台账已清: 后续 pump 无重复 notice
        layer.pump();
        assert!(layer.take_notices().is_empty(), "无重复 notice");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn disable_event_visible_to_gui_pump() {
        let dir = temp_scan("disable");
        let mut layer = ServiceLayer::bootstrap(&dir).expect("bootstrap");
        if !wait_until(
            || {
                layer.pump();
                layer.states.get(&layer.core_id) == Some(&ServiceState::Available)
            },
            Duration::from_secs(3),
        ) {
            // 诊断: 启动状态事件未达 — 区分"发布前订阅未就位(事件丢)"vs"慢"
            let trace: Vec<String> = layer
                .manager
                .trace_entries()
                .iter()
                .filter(|t| t.kind == KIND_STATE_CHANGE)
                .map(|t| format!("{:?}:{}", t.direction, t.kind))
                .collect();
            let dead: Vec<String> = layer
                .manager
                .dead_letters()
                .iter()
                .map(|d| format!("{}:{:?}", d.kind, d.reason))
                .collect();
            panic!(
                "core.fake 启动 Available 事件未达 (3s): state_sent={trace:?} \
                 dead={dead:?} seen={} states={:?}",
                layer.events_seen, layer.states
            );
        }
        let seen_before = layer.events_seen;

        layer.manager.disable(layer.core_id).expect("disable");
        if !wait_until(
            || {
                layer.pump();
                layer.states.get(&layer.core_id) == Some(&ServiceState::Offline)
            },
            Duration::from_secs(3),
        ) {
            // 诊断: Offline 事件未达 — state_sent 含补发/启动/本次, 分辨卡点
            let trace: Vec<String> = layer
                .manager
                .trace_entries()
                .iter()
                .filter(|t| t.kind == KIND_STATE_CHANGE)
                .map(|t| format!("{:?}:{}", t.direction, t.kind))
                .collect();
            let dead: Vec<String> = layer
                .manager
                .dead_letters()
                .iter()
                .map(|d| format!("{}:{:?}", d.kind, d.reason))
                .collect();
            panic!(
                "disable Offline 事件未达 (3s): state_sent={trace:?} dead={dead:?} \
                 seen={} states={:?}",
                layer.events_seen, layer.states
            );
        }
        assert!(layer.events_seen > seen_before, "新事件被消费");
        std::fs::remove_dir_all(&dir).ok();
    }
}
