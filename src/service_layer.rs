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
    ActionInvoke, GitOpFinished, OpError as SolOpError, OpResult as SolOpResult, RepoState,
    SolutionContract, TreeChanged as SolTreeChanged, TreeSnapshot, WatchReq as SolWatchReq,
    KIND_ACTION as KIND_SOL_ACTION, KIND_GIT_OP_FINISHED, KIND_LOAD,
    KIND_OP_REPLY as KIND_SOLUTION_REPLY, KIND_TREE_CHANGED as KIND_SOLUTION_CHANGED,
    KIND_WATCH as KIND_SOLUTION_WATCH,
};
use solution_service::service::solution_service_factory;
// change asset-service task 3.4: 右槽资产域服务 (design D1)。
use asset_service::contract::{
    AssetContract, AssetRegistration, RefreshReq as AssetRefreshReq,
    TreeChanged as AstTreeChanged, TreeSnapshot as AstTreeSnapshot,
    WatchReq as AssetWatchReq, KIND_REFRESH as KIND_ASSET_REFRESH,
    KIND_ACTION as KIND_ASSET_ACTION, KIND_REGISTER as KIND_ASSET_REGISTER, KIND_TREE_CHANGED as KIND_ASSET_CHANGED,
    KIND_WATCH as KIND_ASSET_WATCH,
};
use asset_service::service::asset_service_factory;
use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::Path;

/// 入失败 notice 队列 (design D7: 路由键 = action_id, 文案只显示)。
///
/// **自由函数而非 `&mut self` 方法**：调用点在 `pump` 的
/// `self.listener.drain()` 循环内 — `&mut self` 方法会与 listener 的可变
/// 借用冲突；借用**单个字段** `notices` 则与 `listener` 不相交，合法。
fn push_notice(
    notices: &mut Vec<(Option<String>, String)>,
    action: Option<&str>,
    text: String,
) {
    notices.push((action.map(str::to_string), text));
}

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
    /// `sessions_changed` 投影镜像（task 3.1；solution-overview/active 渲染源）。
    pub sessions_view: Option<SessionView>,
    /// 行徽标数据（change solution-git-sync D4）：最近一次 git 结果检视
    /// 过的每个仓 path → state；键经 `badge_key` 归一（反斜杠/正斜杠）。
    pub repo_status: HashMap<String, RepoState>,
    /// 待消费帧（task 3.1；app 泵转发给 AgentTerminal.feed）。
    frames: Vec<FrameBytes>,
    /// 失败 notice 队列（spec delta: 点名失败操作）。
    ///
    /// **(action_id, 显示文案)** — 路由用前者、显示用后者 (design D7)。
    /// `action = None` = 非动作触发的通用通知（agent 控制回执、无配对的
    /// route error）。
    notices: Vec<(Option<String>, String)>,
    /// 已发控制台账: corr -> (操作, 标签)（回执/route_error 配对）。
    pending_ops: HashMap<Corr, (CtrlOp, &'static str)>,
    /// 控制成功回执（task 3.2: app 据此切 console tab — 成功才切，spec delta）。
    successes: Vec<CtrlOp>,
    /// solution 操作成功回执的 **action_id**（非 load 的 Ok 配对；表单签
    /// 自关消费 — design D7: 路由按 id 不按文案）。
    sol_successes: Vec<String>,
    /// watch 握手已发出（bootstrap 时 agent 尚 loading 会被 fail-fast 拒 →
    /// 首个 Available 事件到达时补发，task 3.1）。
    watch_sent: bool,
    /// agent 就绪一次性信号 (pump 见 Available 置位; app 据此补推主题 —
    /// 服务快照默认深色, 宿主主题为准 — task 3.2)。
    agent_ready: bool,
    /// solution 域服务实例 id（change solution-service task 2.2；树路由目标）。
    pub solution_id: InstanceId,
    /// 资产域服务实例 id（change asset-service task 3.4；注册/刷新/状态
    /// 路由目标 —— 见组 4 的 `asset.register`）。
    #[allow(dead_code)] // 组 4 接线 `asset.register` 后读取（同 agent_id 先例）
    pub asset_id: InstanceId,
    /// asset watch 握手已发出（同 agent/solution 的 loading 规避时序；
    /// 缺它则服务 `emit` 无订阅者 ⇒ 镜像永不更新 —— change asset-service 4.3）。
    asset_watch_sent: bool,
    /// 测试观测面：`asset.register` 发送次数 —— 用于钉死
    /// 「交树投影未变 ⇒ 零注册」的 O(1) 守卫（spec: Data-change
    /// notification is O(1) … no per-node traversal）。
    #[cfg(test)]
    pub(crate) asset_register_sends: usize,
    /// 测试观测面：`asset.refresh` 发送次数（⟳ 在途是否仍发 = spec
    /// *Explicit refresh still restarts* 的 GUI 侧判据）。
    #[cfg(test)]
    pub(crate) asset_refresh_sends: usize,
    /// 测试观测面：`asset.action` 发送次数。
    ///
    /// asset 的两个动作全部 `ExecutedBy::View` ⇒ 视图本地执行、**不发
    /// invoke** ⇒ 本计数恒 0（task 6.2 的不变量）。[`Self::submit_asset_action`]
    /// 是服务侧动作未来的**唯一写入口** —— 谁加 invoke 谁就得递增它，
    /// 于是那条断言会真的报警。
    #[cfg(test)]
    pub(crate) asset_action_sends: usize,
    /// 测试观测面：`solution.action` 发送次数 —— task 5.1 会话前置门
    /// （design D3 路线 A）的判据：Running 拦下 ⇒ 本计数不增；
    /// Exited 放行 ⇒ 增 1。[`Self::submit_action`] 是 solution 动作的
    /// 唯一写入口。
    #[cfg(test)]
    pub(crate) sol_action_sends: usize,
    /// 项目树镜像（`tree_changed` 快照 + 操作回执自带快照；视图/交树渲染源）。
    tree_view: Option<TreeSnapshot>,
    /// **资产视图镜像**（change asset-service task 4.2）：`asset.tree_changed`
    /// 快照 —— `AssetOverview` 的渲染源（服务拥有状态, GUI 只读镜像, 同构
    /// solution 的 `tree_view`）。
    asset_view: Option<AstTreeSnapshot>,
    /// 测试注入覆盖 (优先于真实镜像 — 无头测试不经真实 load 时的视图
    /// 数据源; 服务首推空快照不得踩掉注入值)。
    #[cfg(test)]
    tree_view_override: Option<TreeSnapshot>,
    /// solution watch 握手已发出（同 agent 的 loading 规避时序，task 2.2）。
    solution_watch_sent: bool,
    /// solution 操作台账: corr -> (action_id, 标签)（回执/route_error 配对）。
    /// design D7: **id 用于路由, label 只进显示文案** — 改文案不得改行为。
    pending_sol_ops: HashMap<Corr, (String, String)>,
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
        // change asset-service task 3.4: 资产域服务 (design D1)。
        // 注册/枚举/三态归服务端, GUI 只持镜像 (design D2)。
        manager.register_contract("asset", ContractId::of::<AssetContract>());
        manager
            .register_internal(
                "asset",
                asset_service_factory,
                ContractId::of::<AssetContract>(),
            )
            .map_err(|e| format!("register asset: {e}"))?;
        let asset_id = manager
            .load(&"asset".into())
            .map_err(|e| format!("load asset: {e}"))?;
        // GUI = 普通参与者: 自己的 mailbox + 订阅 (= 往路由表加规则, D7/D9)
        // watch 握手不在这里发: 刚 load 的实例仍 loading, tell 会被 fail-fast
        // 拒 (target not available) — 由 pump 见 agent Available 事件时补发。
        let listener = manager.new_mailbox(64);
        manager.subscribe_state(core_id, &listener);
        manager.subscribe_state(agent_id, &listener);
        manager.subscribe_state(solution_id, &listener);
        manager.subscribe_state(asset_id, &listener);
        // 订阅时序补给 (竞态修复): driver 的 InitOk 翻转是异步任务, 可能抢在
        // subscribe_state 之前发布 — 订阅表还空, 启动状态事件被静默丢弃
        // (实测: 高负载并行下 watch 握手因此断链, 会话投影永不产生)。
        // 订阅后按快照补发等价事件: 与真事件同形 (from=Loading), 消费方幂等
        // (watch/agent_ready 均一次性守卫), 重复到达无害。
        for (id, name) in [
            (core_id, "core.fake"),
            (agent_id, "agent"),
            (solution_id, "solution"),
            (asset_id, "asset"),
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
            repo_status: HashMap::new(),
            frames: Vec::new(),
            notices: Vec::new(),
            pending_ops: HashMap::new(),
            successes: Vec::new(),
            sol_successes: Vec::new(),
            watch_sent: false,
            asset_watch_sent: false,
            #[cfg(test)]
            asset_register_sends: 0,
            #[cfg(test)]
            asset_refresh_sends: 0,
            #[cfg(test)]
            asset_action_sends: 0,
            #[cfg(test)]
            sol_action_sends: 0,
            agent_ready: false,
            solution_id,
            asset_id,
            tree_view: None,
            asset_view: None,
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
                        // change asset-service 4.3: asset watch 握手同款时序
                        if !self.asset_watch_sent
                            && change.instance == self.asset_id
                            && change.to == ServiceState::Available
                        {
                            let watch = Envelope::new(
                                Route::Instance(self.asset_id),
                                watch_from.clone(),
                                KIND_ASSET_WATCH,
                                Payload::new(AssetWatchReq),
                            );
                            if self.manager.tell(watch).is_ok() {
                                self.asset_watch_sent = true;
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
                            push_notice(&mut self.notices, None, format!("{label}失败: {msg}"));
                        }
                        None => {
                            // solution 动作台账 (design D7): route_error 与回执
                            // 同款配对 —— 按 action_id 点名 (不落笼统文案),
                            // 台账同步销账 (不悬挂泄漏)。
                            if let Some((action_id, label)) =
                                env.corr.and_then(|c| self.pending_sol_ops.remove(&c))
                            {
                                push_notice(
                                    &mut self.notices,
                                    Some(action_id.as_str()),
                                    format!("{label}失败: {msg}"),
                                );
                            } else if let Some(c) =
                                env.corr.filter(|c| self.pending_loads.remove(c))
                            {
                                // load 链路: 路由失败也进回执队列 (打开流程统一收口)
                                self.load_results.push((
                                    c,
                                    Err(SolOpError::Persist(format!(
                                        "solution 服务请求失败: {msg}"
                                    ))),
                                ));
                            } else {
                                push_notice(&mut self.notices, None, format!("服务请求失败: {msg}"));
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
                            Some(err) => push_notice(&mut self.notices, None, format!("{label}失败: {err}")),
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
                // ---- change asset-service 4.2: 资产视图镜像 ----
                KIND_ASSET_CHANGED => {
                    self.events_seen += 1;
                    if let Ok(changed) = env.downcast::<AstTreeChanged>() {
                        self.asset_view = Some(changed.0.clone());
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
                                if let Some((action_id, label)) = &paired {
                                    // 路由键 = action_id, label 只进文案 (D7)
                                    push_notice(&mut self.notices, Some(action_id.as_str()), format!("{label}失败: {e}"));
                                }
                            } else if let Some((action_id, _)) = &paired {
                                // 非 load 的 Ok 配对 → 成功队列 (表单签自关)
                                self.sol_successes.push(action_id.clone());
                            }
                        }
                        Err(_) => {
                            if let Some((action_id, label)) = &paired {
                                push_notice(
                                    &mut self.notices,
                                    Some(action_id.as_str()),
                                    format!("{label}失败: 回执类型不匹配"),
                                );
                            }
                        }
                    }
                }
                // ---- change solution-git-worktree 5.2: git 操作完成事件 ----
                // 无 corr 的 tell (design D1) —— 按 kind 直取, 不进销账表,
                // 迟到也无死信噪音; 按 action_id 入 notice (node-actions:
                // 按 id 路由不按 label)。
                KIND_GIT_OP_FINISHED => {
                    self.events_seen += 1;
                    if let Ok(done) = env.downcast::<GitOpFinished>() {
                        // 行徽标数据 (solution-git-sync D4): 本次运行检视过
                        // 的每个仓 → path→state (键经 badge_key 归一)
                        for st in &done.statuses {
                            self.repo_status.insert(
                                solution_service::contract::badge_key(&st.path),
                                st.state,
                            );
                        }
                        // 文案用 label (solution-git-sync D5, 未知 id 回退裸
                        // id); 成功且带 detail（merge 摘要）→ 一并上屏
                        // (spec: completion notice reports the merge result)
                        let name = solution_service::contract::action_label(&done.action_id)
                            .unwrap_or(done.action_id.as_str());
                        let text = if done.ok {
                            if done.detail.is_empty() {
                                format!("{name}完成: {}", done.target)
                            } else {
                                format!("{name}完成: {} — {}", done.target, done.detail)
                            }
                        } else {
                            format!("{name}失败: {}", done.detail)
                        };
                        push_notice(&mut self.notices, Some(done.action_id.as_str()), text);
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

    /// solution 动作 ask 通道（design D6 写入口单点 / D7 路由键）：
    /// 统一发 `solution.action`，台账记 `(action_id, label)` — 回执配对后
    /// **按 id 路由、label 只进显示文案**（改文案不得改行为）。
    /// 回执/route_error 由 `pump` 配对，失败入 notice；成功回执自带树快照
    /// 进镜像。
    pub fn submit_action(
        &mut self,
        action_id: &str,
        node: Option<String>,
        args: Option<Payload>,
        label: &str,
    ) -> Result<Corr, service_manager::RouteError> {
        #[cfg(test)]
        {
            self.sol_action_sends += 1;
        }
        let corr = self.listener.submit(
            Route::Service("solution".into()),
            KIND_SOL_ACTION,
            Payload::new(ActionInvoke {
                node,
                action_id: action_id.to_string(),
                args,
            }),
        )?;
        self.pending_sol_ops
            .insert(corr, (action_id.to_string(), label.to_string()));
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

    /// 某仓的行徽标状态（None = 未观测 → 视图渲染未知灰）。
    pub fn repo_badge(&self, path: &str) -> Option<RepoState> {
        self.repo_status
            .get(&solution_service::contract::badge_key(path))
            .copied()
    }

    /// **资产视图镜像**（change asset-service task 4.2）——
    /// `AssetOverview` 的唯一渲染源（服务发布, 本层只收不造）。
    #[allow(dead_code)] // 任务 4.4 面板改读镜像后消费
    pub fn asset_view(&self) -> Option<&AstTreeSnapshot> {
        self.asset_view.as_ref()
    }

    /// 全量注册 ask（change asset-service 4.3）—— 取代对 `AssetManager`
    /// 的直调。**提交失败（服务尚未 Available）返回 Err 且不记状态**，
    /// 调用方下次 pump 重试（`sync_asset_registration` 的 `last_handoff`
    /// 守卫按"成功才记"实现 ⇒ 天然自愈）。
    pub fn submit_asset_register(
        &mut self,
        reg: AssetRegistration,
    ) -> Result<Corr, service_manager::RouteError> {
        #[cfg(test)]
        {
            self.asset_register_sends += 1;
        }
        self.listener.submit(
            Route::Service("asset".into()),
            KIND_ASSET_REGISTER,
            Payload::new(reg),
        )
    }

    /// 显式刷新 ask（⟳；supersede 在途、重读 filter 源）。
    pub fn submit_asset_refresh(
        &mut self,
    ) -> Result<Corr, service_manager::RouteError> {
        #[cfg(test)]
        {
            self.asset_refresh_sends += 1;
        }
        self.listener.submit(
            Route::Service("asset".into()),
            KIND_ASSET_REFRESH,
            Payload::new(AssetRefreshReq),
        )
    }

    /// 服务侧资产动作的 ask 通道（`KIND_ASSET_ACTION`）。
    ///
    /// **当前无调用方**：asset 的两个动作都标 `ExecutedBy::View`，视图
    /// 本地执行即可（design D5）。保留为未来服务侧动作的**唯一写入口**，
    /// 好让 `asset_action_sends == 0` 这条不变量有确定的递增点。
    #[allow(dead_code)] // 服务侧资产动作落地后由视图调用
    pub fn submit_asset_action(
        &mut self,
        node: Option<String>,
        action_id: String,
    ) -> Result<Corr, service_manager::RouteError> {
        #[cfg(test)]
        {
            self.asset_action_sends += 1;
        }
        self.listener.submit(
            Route::Service("asset".into()),
            KIND_ASSET_ACTION,
            Payload::new(asset_service::contract::ActionInvoke { node, action_id }),
        )
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

    /// 取走失败 notice `(action_id, 文案)`（app 消费 → `app.notice`;
    /// action_id 供表单签行内错误路由 — design D7）。
    #[allow(dead_code)] // task 3.2 app.notice 接线
    pub fn take_notices(&mut self) -> Vec<(Option<String>, String)> {
        std::mem::take(&mut self.notices)
    }



    /// 取走控制成功回执（app 据此切 console tab；失败已入 notice）。
    pub fn take_successes(&mut self) -> Vec<CtrlOp> {
        std::mem::take(&mut self.successes)
    }

    /// 取走 solution 操作成功回执的 **action_id**（表单签自关消费 —
    /// design D7: 按 id 配对, 不按 label 文案）。
    pub fn take_sol_successes(&mut self) -> Vec<String> {
        std::mem::take(&mut self.sol_successes)
    }

    /// 一次性: agent 首次就绪 (app 据此推主题快照)。
    pub fn take_agent_ready(&mut self) -> bool {
        std::mem::take(&mut self.agent_ready)
    }

    /// 当前会话投影镜像（solution-overview/active 渲染源）。
    #[allow(dead_code)] // task 3.2 solution-overview/active 接线
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
        assert!(layer.manager.has_service(&"asset".into()), "Catalog 登记 asset");
        assert_eq!(
            layer.manager.instance_count(),
            4,
            "core.fake + agent + solution + asset 均已装载"
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

    /// change asset-service task 3.4: 资产域服务注册/装载 + 状态翻转到达
    /// pump。（验证替代 `cargo run`：无头环境无法断言 GUI 内的 running。）
    #[test]
    fn asset_service_loads_and_reports_state() {
        let dir = temp_scan("asset-boot");
        let mut layer = ServiceLayer::bootstrap(&dir).expect("bootstrap");

        assert!(layer.manager.has_service(&"asset".into()), "Catalog 登记 asset");
        assert_eq!(
            layer.manager.instance_count(),
            4,
            "core.fake + agent + solution + asset 均已装载"
        );
        assert!(
            wait_until(
                || {
                    layer.pump();
                    layer.states.get(&layer.asset_id) == Some(&ServiceState::Available)
                },
                Duration::from_secs(3)
            ),
            "asset 状态翻转经订阅到达 GUI mailbox: {:?}",
            layer.states
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// change asset-service task 4.2: `asset.tree_changed` 收包 → 镜像可读
    /// 且内容完整（**端到端**：真实服务真枚举、经 watch 推达）。
    #[test]
    fn asset_tree_changed_fills_the_mirror() {
        use asset_service::contract::{
            AssetRegistration, AssetState, ModuleRegistration, ProjectRegistration, WatchReq,
            KIND_WATCH,
        };

        let dir = temp_scan("asset-mirror");
        let proj = dir.join("proj");
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::write(proj.join("main.rs"), "fn main() {}").unwrap();

        let mut layer = ServiceLayer::bootstrap(&dir).expect("bootstrap");
        assert!(
            wait_until(
                || {
                    layer.pump();
                    layer.states.get(&layer.asset_id) == Some(&ServiceState::Available)
                },
                Duration::from_secs(3)
            ),
            "asset 服务应可用"
        );

        // watch 握手（`from` = GUI mailbox ⇒ 服务记为订阅者）
        let watch = Envelope::new(
            Route::Instance(layer.asset_id),
            layer.listener.address().clone(),
            KIND_WATCH,
            Payload::new(WatchReq),
        );
        layer.manager.tell(watch).expect("watch tell");

        // 全量注册一个真实项目根
        let reg = AssetRegistration {
            workspace_dir: dir.clone(),
            projects: vec![ProjectRegistration {
                name: "proj".into(),
                root: proj.clone(),
                is_git: false,
                // 纯结构 — 过滤集由服务按 is_git+root 现场供给 (design D3)
                modules: vec![ModuleRegistration {
                    name: "proj".into(),
                    root: proj.clone(),
                    children: Vec::new(),
                }],
            }],
        };
        layer
            .listener
            .submit(
                Route::Service("asset".into()),
                asset_service::contract::KIND_REGISTER,
                Payload::new(reg),
            )
            .expect("register submit");

        // 枚举落地 → tell → pump → 镜像
        assert!(
            wait_until(
                || {
                    layer.pump();
                    matches!(
                        layer.asset_view().map(|v| &v.state),
                        Some(AssetState::Forest { .. })
                    )
                },
                Duration::from_secs(5)
            ),
            "枚举落地后镜像应可读, 当前 = {:?}",
            layer.asset_view().map(|v| &v.state)
        );

        let snap = layer.asset_view().expect("镜像存在");
        match &snap.state {
            AssetState::Forest { nodes } => {
                assert_eq!(nodes.len(), 1, "根节点数 = 注册的项目数");
                assert_eq!(nodes[0].label, "proj");
                assert!(nodes[0].id.ends_with("proj"), "根 = 项目目录");
                let mut found = false;
                fn walk(n: &asset_service::contract::AssetNode, hit: &mut bool) {
                    if n.label == "main.rs" {
                        *hit = true;
                    }
                    for c in &n.children {
                        walk(c, hit);
                    }
                }
                for root_node in nodes {
                    walk(root_node, &mut found);
                }
                assert!(
                    found,
                    "内容完整 — 枚举产物应含 main.rs, 实为 {:?}",
                    nodes[0].children
                );
            }
            other => panic!("应为 Forest, 实为 {other:?}"),
        }
        assert!(snap.generation >= 2, "失效 +1、落地 +1");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// change agent-service task 1.3: agent 薄服务注册/装载 + 状态事件到 pump。
    #[test]
    fn agent_service_loads_and_reports_state() {
        let dir = temp_scan("agent-boot");
        let mut layer = ServiceLayer::bootstrap(&dir).expect("bootstrap");
        assert!(layer.manager.has_service(&"agent".into()), "Catalog 登记 agent");
        assert_eq!(layer.manager.instance_count(), 4, "core.fake + agent + solution + asset");
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
            ConfigureReq, CreateReq, LoadReq, ACTION_CONFIGURE_REGISTER, ACTION_CREATE,
        };

        let dir = temp_scan("sol-mirror");
        let mut layer = ServiceLayer::bootstrap(&dir).expect("bootstrap");
        assert!(
            layer.manager.has_service(&"solution".into()),
            "Catalog 登记 solution"
        );
        assert_eq!(
            layer.manager.instance_count(),
            4,
            "core.fake + agent + solution + asset"
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

        // 绑定工作区 (design D5): create 的落盘目标来自 load 绑定,
        // 不再随请求带 workspace_dir — 不先 load 就 create 会拿到
        // "no workspace bound"。
        let load_corr = layer
            .submit_solution_load(Payload::new(LoadReq {
                workspace_dir: dir.clone(),
            }))
            .expect("submit load");
        assert!(
            wait_until(
                || {
                    layer.pump();
                    layer.take_load_result(load_corr).is_some()
                },
                Duration::from_secs(3)
            ),
            "load 落定 (绑定)"
        );

        // create: 真实项目目录（simple item 的 path 必须存在）
        let proj = dir.join("proj");
        std::fs::create_dir_all(&proj).unwrap();
        layer
            .submit_action(
                ACTION_CREATE,
                None,
                Some(Payload::new(CreateReq { path: proj.clone() })),
                "创建 solution",
            )
            .expect("submit create");
        assert!(
            wait_until(
                || {
                    layer.pump();
                    matches!(layer.tree_view, Some(ref t) if t.tree().projects.len() == 1)
                },
                Duration::from_secs(3)
            ),
            "create 后树变更通知到达且镜像更新"
        );
        assert_eq!(
            layer.tree_view.as_ref().unwrap().handoff().projects.len(),
            1,
            "默认双 true: 交树含新 project"
        );

        // configure: 关注册 → handoff 收缩, 全量集保留 (spec: Asset
        // registration flag gates project handoff)
        layer
            .submit_action(
                ACTION_CONFIGURE_REGISTER,
                // 目标单源 = node (design D4); 值在 args
                Some(solution_service::solution_node_id(&proj)),
                Some(Payload::new(ConfigureReq {
                    register_to_asset_manager: Some(false),
                    git_enabled: None,
                })),
                "配置 solution",
            )
            .expect("submit configure");
        assert!(
            wait_until(
                || {
                    layer.pump();
                    matches!(
                        layer.tree_view,
                        Some(ref t) if t.handoff().projects.is_empty() && t.tree().projects.len() == 1
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
            notices[0].1.contains("切换会话"),
            "notice 点名操作: {notices:?}"
        );
        assert!(
            notices[0].1.contains("no such session"),
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

    /// change solution-git-worktree 5.2: `KIND_GIT_OP_FINISHED` 收包 →
    /// 成功/失败两态各出一条以 action_id 命名的 notice；无 corr 事件
    /// 不进销账表 → 迟到无死信噪音 (design D1)。
    #[test]
    fn git_op_finished_produces_notice_without_dead_letter() {
        use solution_service::contract::{GitOpFinished, KIND_GIT_OP_FINISHED};

        let dir = temp_scan("git-finished");
        let mut layer = ServiceLayer::bootstrap(&dir).expect("bootstrap");
        assert!(
            wait_until(
                || {
                    layer.pump();
                    layer.states.get(&layer.solution_id) == Some(&ServiceState::Available)
                },
                Duration::from_secs(3)
            ),
            "solution 服务应可用"
        );

        let inject = |layer: &mut ServiceLayer, done: GitOpFinished| {
            layer
                .manager
                .tell(Envelope::new(
                    Route::Instance(layer.listener.id()),
                    Address::instance(layer.solution_id),
                    KIND_GIT_OP_FINISHED,
                    Payload::new(done),
                ))
                .expect("inject git op finished");
        };

        // 成功态 → notice 点名 action + 目标
        inject(
            &mut layer,
            GitOpFinished {
                action_id: "git.pull".into(),
                target: "/ws/proj".into(),
                ok: true,
                detail: String::new(),
                statuses: Vec::new(),
            },
        );
        assert!(
            wait_until(
                || {
                    layer.pump();
                    !layer.notices.is_empty()
                },
                Duration::from_secs(3)
            ),
            "成功事件应出 notice"
        );
        let (action, text) = layer.take_notices().pop().expect("notice 在场");
        assert_eq!(action.as_deref(), Some("git.pull"), "路由键 = action_id");
        assert!(
            text.starts_with("拉取完成") && text.contains("/ws/proj"),
            "文案用 label (D5): {text}"
        );

        // 成功带 detail（merge 摘要）→ 一并上屏 (spec: reports the result)
        inject(
            &mut layer,
            GitOpFinished {
                action_id: "git.pull".into(),
                target: "/ws/proj".into(),
                ok: true,
                detail: "Fast-forward".into(),
                statuses: Vec::new(),
            },
        );
        assert!(
            wait_until(
                || {
                    layer.pump();
                    !layer.notices.is_empty()
                },
                Duration::from_secs(3)
            ),
            "带 detail 的成功事件应出 notice"
        );
        let (_, text) = layer.take_notices().pop().expect("notice 在场");
        assert!(text.contains("Fast-forward"), "成功 detail 上屏: {text}");

        // 未知 id → 回退裸 id（未来新动作不崩文案）
        inject(
            &mut layer,
            GitOpFinished {
                action_id: "git.future_op".into(),
                target: "/ws/proj".into(),
                ok: false,
                detail: "boom".into(),
                statuses: Vec::new(),
            },
        );
        assert!(
            wait_until(
                || {
                    layer.pump();
                    !layer.notices.is_empty()
                },
                Duration::from_secs(3)
            ),
            "未知 id 事件应出 notice"
        );
        let (_, text) = layer.take_notices().pop().expect("notice 在场");
        assert!(
            text.starts_with("git.future_op失败"),
            "未知 id 回退裸 id: {text}"
        );

        // 失败态 → notice 点名 action + stderr 摘要
        inject(
            &mut layer,
            GitOpFinished {
                action_id: "git.push".into(),
                target: "/ws/proj".into(),
                ok: false,
                detail: "remote rejected: non-fast-forward".into(),
                statuses: Vec::new(),
            },
        );
        assert!(
            wait_until(
                || {
                    layer.pump();
                    !layer.notices.is_empty()
                },
                Duration::from_secs(3)
            ),
            "失败事件应出 notice"
        );
        let (action, text) = layer.take_notices().pop().expect("notice 在场");
        assert_eq!(action.as_deref(), Some("git.push"));
        assert!(
            text.starts_with("推送失败") && text.contains("non-fast-forward"),
            "文案用 label (D5): {text}"
        );

        // 迟到/无 corr: 事件不进 pending 表 → 零死信 (design D1)
        let dead_before = layer.manager.dead_letters().len();
        inject(
            &mut layer,
            GitOpFinished {
                action_id: "git.pull".into(),
                target: "/ws/proj".into(),
                ok: true,
                detail: String::new(),
                statuses: Vec::new(),
            },
        );
        assert!(
            wait_until(
                || {
                    layer.pump();
                    !layer.notices.is_empty()
                },
                Duration::from_secs(3)
            ),
            "重复事件照常出 notice"
        );
        layer.take_notices();
        assert_eq!(
            layer.manager.dead_letters().len(),
            dead_before,
            "无 corr 事件不得产生死信噪音"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// change solution-git-worktree (route_error 配对补全): solution 动作在途
    /// 收到 `KIND_ROUTE_ERROR`（超时/路由失败/reject）→ 与回执同款按
    /// action_id 点名出 notice（不落笼统文案）且 `pending_sol_ops` 台账
    /// 同步销账（design D7: id 路由, label 只进文案）；load 台账与无 corr
    /// 两支不受影响。
    ///
    /// 投递保真：带 corr 的 `tell` 走 router `complete()` 销账 → 按 pending
    /// 表投进提交者邮箱 —— 与真实超时路径（5s 定时器 `tx.try_send`）同一条
    /// 链。故先发**真 ask** 造出 pending（载荷类型对不上 → 服务第一支
    /// `reject` 静默丢弃, 不抢先回执），再合成 route_error 完成它；不等 5s。
    #[test]
    fn route_error_pairs_solution_action_ledger() {
        let dir = temp_scan("route-error-sol");
        let mut layer = ServiceLayer::bootstrap(&dir).expect("bootstrap");
        assert!(
            wait_until(
                || {
                    layer.pump();
                    layer.states.get(&layer.solution_id) == Some(&ServiceState::Available)
                },
                Duration::from_secs(3)
            ),
            "solution 服务应可用"
        );
        layer.take_notices();

        let ask = |layer: &mut ServiceLayer, kind: &'static str| -> Corr {
            layer
                .listener
                .submit(
                    Route::Service("solution".into()),
                    kind,
                    Payload::new(CorePing(1)), // 类型对不上 → 服务 reject, pending 留存
                )
                .expect("ask 应急切成功 (服务 Available)")
        };
        let inject_error = |layer: &mut ServiceLayer, corr: Option<Corr>| {
            let env = Envelope::new(
                Route::Instance(layer.listener.id()),
                Address::instance(layer.solution_id),
                KIND_ROUTE_ERROR,
                Payload::new(service_manager::RouteError::Timeout),
            );
            let env = match corr {
                Some(c) => env.with_corr(c),
                None => env,
            };
            layer.manager.tell(env).expect("route_error 投递");
        };

        // 支 1: solution 动作台账 → 点名 notice + 台账销账
        let corr = ask(&mut layer, KIND_SOL_ACTION);
        layer
            .pending_sol_ops
            .insert(corr, ("git.pull".into(), "拉取".into()));
        inject_error(&mut layer, Some(corr));
        assert!(
            wait_until(
                || {
                    layer.pump();
                    !layer.notices.is_empty()
                },
                Duration::from_secs(3)
            ),
            "route_error 应出点名 notice"
        );
        let (action, text) = layer.take_notices().pop().expect("notice 在场");
        assert_eq!(
            action.as_deref(),
            Some("git.pull"),
            "路由键 = action_id (D7)"
        );
        assert!(
            text.contains("拉取失败") && text.contains("timed out"),
            "{text}"
        );
        assert!(
            layer.pending_sol_ops.is_empty(),
            "route_error 配对后台账不悬挂 (防泄漏)"
        );

        // 支 2: load 台账 → 回执队列收 Err, 不出 notice (打开链路统一收口)
        let load_corr = layer
            .submit_solution_load(Payload::new(CorePing(1)))
            .expect("load ask 应急切成功");
        inject_error(&mut layer, Some(load_corr));
        assert!(
            wait_until(
                || {
                    layer.pump();
                    layer.load_results.iter().any(|(c, _)| *c == load_corr)
                },
                Duration::from_secs(3)
            ),
            "load 链路 route_error 应进回执队列"
        );
        assert!(layer.take_notices().is_empty(), "load 支不出 notice");
        match layer.take_load_result(load_corr) {
            Some(Err(SolOpError::Persist(t))) => {
                assert!(t.contains("solution 服务请求失败"), "{t}")
            }
            other => panic!("load 回执应是 Persist 错, 实际: {:?}", other.map(|_| "ok")),
        }

        // 支 3: 无 corr → 兜底文案 (action = None), 不误配台账
        inject_error(&mut layer, None);
        assert!(
            wait_until(
                || {
                    layer.pump();
                    !layer.notices.is_empty()
                },
                Duration::from_secs(3)
            ),
            "无 corr route_error 应出兜底 notice"
        );
        let (action, text) = layer.take_notices().pop().expect("notice 在场");
        assert_eq!(action, None, "无 corr 无路由键");
        assert!(text.starts_with("服务请求失败"), "{text}");

        std::fs::remove_dir_all(&dir).ok();
    }
}
