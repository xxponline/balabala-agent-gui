//! ServiceManager 门面：Catalog/实例表/reload（task 3.1 范围；
//! runtime/Router/Supervisor/Loader 由 task 3.4/4.x/3.3 逐个接入本结构）。

use crate::catalog::{Catalog, ServiceDescriptor, ServiceOrigin};
use crate::instance::{Instances, Participant};
use crate::listener::Listener;
use crate::router::{DeadLetterRecord, Router, TraceEntry};
use crate::runtime::LayerRuntime;
use crate::scanner::{self, ScanError};
use crate::supervisor::Supervisor;
use service_core::{
    Address, ContractId, Envelope, InstanceId, Messaging, Payload, RouteError, ServiceFactory,
    ServiceName,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

/// ask 默认超时（Router 持 deadline，D6）。
const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(5);

/// 宿主构建指纹（build.rs 固化，与插件侧同一算法）——Loader 装载前比对，
/// 不匹配**显式拒载**（task 3.2 / design D8、D12 路线甲）。
pub const HOST_FINGERPRINT: &str = env!("SERVICE_HOST_FINGERPRINT");

/// 服务管理中间层门面（对宿主暴露，**不泄漏 tokio/kameo 类型**，D9）。
pub struct ServiceManager {
    /// 服务扫描目录（= plugins/ 落点，D8 静态扫描、无 watch）。
    scan_dir: PathBuf,
    /// 内部服务显式注册（持久，reload 后重新并入，程序注册优先于同名 DLL）。
    internals: Mutex<Vec<ServiceDescriptor>>,
    /// Catalog：唯一被 reload 改写的共享状态。
    catalog: RwLock<Catalog>,
    /// 实例/参与者表：与 Catalog **结构分离**，reload 不触碰（D8）。
    pub(crate) instances: Arc<Mutex<Instances>>,
    /// 每实例状态机（task 3.4）：唯一恢复权威，kameo 自动 restart 禁用（D7）。
    pub(crate) supervisor: Arc<Supervisor>,
    /// 参与者 id 分配（服务实例与宿主参与者共用计数）。
    next_id: AtomicU64,
    /// 中间层自建自持运行时（D9：对外不泄漏 tokio；loader 经 `runtime_token` 取句柄）。
    pub(crate) runtime: LayerRuntime,
    /// 中心路由（全部消息过 Router，D3）。
    router: Arc<Router>,
    /// DLL 库缓存：`ManuallyDrop` ⇒ **永不 dlclose**（D8 真回收 = 进程重启）。
    /// 不驱逐：同路径重载复用首载句柄（产物替换需重启进程才生效）。
    pub(crate) libs: Mutex<std::collections::HashMap<PathBuf, Arc<std::mem::ManuallyDrop<libloading::Library>>>>,
    /// 在跑 driver 的控制柄（每实例至多一个；enable/retry 换新代）。
    pub(crate) drivers: Mutex<std::collections::HashMap<InstanceId, crate::loader::DriverControl>>,
    /// 宿主契约注册表（change `singleton-per-contract`, design D2）：契约名 →
    /// 身份，DLL 装载期按产物契约符号查此表解析（内部服务不经此表，
    /// 注册即携带身份）。同名重复登记且身份不同 = 引导期编程错误 → 断言。
    contracts: Mutex<std::collections::HashMap<ServiceName, ContractId>>,
}

impl ServiceManager {
    /// 创建并执行启动扫描（D8：启动扫一遍建 Catalog）。
    pub fn new(scan_dir: impl Into<PathBuf>) -> Result<Self, ScanError> {
        let runtime = LayerRuntime::new();
        let instances = Arc::new(Mutex::new(Instances::new()));
        let (supervisor, mut events_rx) = Supervisor::new(Arc::clone(&instances));
        let router = Arc::new(Router::new(
            Arc::clone(&instances),
            runtime.handle().clone(),
            DEFAULT_CALL_TIMEOUT,
        ));
        // 状态翻转 → 订阅者转发循环 (订阅塌缩成路由, D7/D9)
        let fwd = router.clone();
        runtime.spawn(async move {
            while let Some(change) = events_rx.recv().await {
                fwd.publish_state_change(change);
            }
        });
        let mgr = ServiceManager {
            scan_dir: scan_dir.into(),
            internals: Mutex::new(Vec::new()),
            catalog: RwLock::new(Catalog::new()),
            instances,
            next_id: AtomicU64::new(1),
            supervisor: Arc::new(supervisor),
            runtime,
            router,
            libs: Mutex::new(std::collections::HashMap::new()),
            drivers: Mutex::new(std::collections::HashMap::new()),
            contracts: Mutex::new(std::collections::HashMap::new()),
        };
        mgr.rebuild_catalog()?;
        Ok(mgr)
    }

    /// 内部服务显式注册：**立即**进 Catalog（与 DLL 条目同表，D8），
    /// 并在每次 reload 时重新并入（程序注册优先于同名 DLL 产物）。
    /// `contract` = 宿主铸造的契约身份（change `singleton-per-contract`, design D1）。
    pub fn register_internal(
        &self,
        name: impl Into<ServiceName>,
        factory: ServiceFactory,
        contract: ContractId,
    ) -> Result<(), ServiceError> {
        let desc = ServiceDescriptor::internal(name, factory, contract);
        {
            let mut internals = self.internals.lock().expect("internals poisoned");
            if internals.iter().any(|d| d.name == desc.name) {
                return Err(ServiceError::DuplicateRegistration(desc.name));
            }
            internals.push(desc.clone());
        }
        let mut catalog = self.catalog.write().expect("catalog poisoned");
        catalog.insert(desc);
        Ok(())
    }

    /// 登记宿主契约表（change `singleton-per-contract`, design D2）：契约名 →
    /// 身份——DLL 装载期按产物契约符号查此表解析；内部服务不经此表。
    ///
    /// 功能：写入/校验一条契约名映射。
    /// 输入：`name` — 契约名（与 DLL 契约符号内容一致）；`contract` — 宿主铸造身份。
    /// 输出：同名同身份幂等；同名异身份 = 引导期编程错误，断言 panic。
    pub fn register_contract(&self, name: impl Into<ServiceName>, contract: ContractId) {
        let mut map = self.contracts.lock().expect("contracts poisoned");
        match map.entry(name.into()) {
            std::collections::hash_map::Entry::Occupied(e) => {
                assert_eq!(
                    *e.get(),
                    contract,
                    "同名契约注册必须声明同一身份（引导期编程错误, design D2）"
                );
            }
            std::collections::hash_map::Entry::Vacant(e) => {
                e.insert(contract);
            }
        }
    }

    /// 契约名 → 身份解析（DLL 装载期用，design D2）。
    ///
    /// 功能：查宿主契约表。
    /// 输入：`name` — DLL 契约符号声明的契约名。
    /// 输出：已登记身份；未登记返回 `None`（装载期显式拒载）。
    pub(crate) fn resolve_contract(&self, name: &ServiceName) -> Option<ContractId> {
        self.contracts
            .lock()
            .expect("contracts poisoned")
            .get(name)
            .copied()
    }

    /// 显式 reload（GUI 按钮/测试调用）：**只影响未来的 load，不动实例表**。
    ///
    /// 合并规则（最小方案）：新扫描条目替换同名旧条目；消失的条目移除
    /// （已 load 实例继续跑，只是不能再 load 新实例）；内部注册最后并入。
    /// 返回 reload 后的条目数。
    pub fn reload_catalog(&self) -> Result<usize, ScanError> {
        self.rebuild_catalog()
    }

    fn rebuild_catalog(&self) -> Result<usize, ScanError> {
        let mut fresh = Catalog::new();
        // 1. 文件系统扫描结果打底
        for desc in scanner::scan(&self.scan_dir)? {
            fresh.insert(desc);
        }
        // 2. 内部注册覆盖同名（程序意图 > 文件系统偶然）
        for desc in self.internals.lock().expect("internals poisoned").iter() {
            fresh.insert(desc.clone());
        }
        let len = fresh.len();
        *self.catalog.write().expect("catalog poisoned") = fresh;
        Ok(len)
    }

    /// 查 Catalog 条目（`load()` 的前置门：查无 → `NoSuchService`，task 3.3）。
    pub fn catalog_entry(&self, name: &ServiceName) -> Option<ServiceDescriptor> {
        self.catalog
            .read()
            .expect("catalog poisoned")
            .get(name)
            .cloned()
    }

    /// 当前 Catalog 全部条目名（有序，测试/观测用）。
    pub fn catalog_names(&self) -> Vec<ServiceName> {
        self.catalog.read().expect("catalog poisoned").names()
    }

    /// 是否存在可 load 的条目（Catalog 门）。
    pub fn has_service(&self, name: &ServiceName) -> bool {
        self.catalog.read().expect("catalog poisoned").contains(name)
    }

    /// origin 便捷访问。
    pub fn origin_of(&self, name: &ServiceName) -> Option<ServiceOrigin> {
        self.catalog_entry(name).map(|d| d.origin)
    }

    /// 分配参与者 id（服务实例与宿主参与者共用；未注册者不存在于表中）。
    pub fn allocate_id(&self) -> InstanceId {
        InstanceId(self.next_id.fetch_add(1, Ordering::Relaxed))
    }

    /// 扫描目录。
    pub fn scan_dir(&self) -> &std::path::Path {
        &self.scan_dir
    }

    /// 当前服务实例数（不含宿主参与者；观测口，GUI/测试用）。
    pub fn instance_count(&self) -> usize {
        self.instances
            .lock()
            .expect("instances poisoned")
            .service_count()
    }

    /// 每实例状态机（task 3.4）。
    pub fn supervisor(&self) -> &Arc<Supervisor> {
        &self.supervisor
    }

    // ---------- 双门 API（D9）：future 门 + poll 门；**无回调门** ----------

    /// ask（future 门，默认超时）：内部急切分发，返回 executor-agnostic
    /// future（gpui executor / tokio / 测试 block_on 均可 await，D9）。
    pub fn call(
        &self,
        env: Envelope,
    ) -> impl Future<Output = Result<Payload, RouteError>> + Send + 'static {
        self.call_with_timeout(env, self.router.default_timeout())
    }

    /// ask（future 门，显式超时）。
    pub fn call_with_timeout(
        &self,
        env: Envelope,
        timeout: Duration,
    ) -> impl Future<Output = Result<Payload, RouteError>> + Send + 'static {
        self.router
            .call_future(env, self.router.self_address(), timeout)
    }

    /// tell：**无同步反馈**（失败落死信，只有观测面可见）。约定：
    /// **GUI 用户触发的操作一律走 ask**（错误可回显），tell 只用于
    /// fire-and-forget 内部流（D6）。
    pub fn tell(&self, env: Envelope) -> Result<(), RouteError> {
        self.router.send(env)
    }

    /// 订阅状态翻转（= 往路由表加一条投递规则，订阅塌缩成路由，D7/D9）。
    pub fn subscribe_state(&self, target: InstanceId, listener: &Listener) {
        self.router.subscribe_state(target, listener.id());
    }

    /// 注册一个宿主参与者（拉模式 actor mailbox）——GUI 就是普通参与者（D3）。
    pub fn new_mailbox(&self, capacity: usize) -> Listener {
        let id = self.allocate_id();
        let (tx, rx) = tokio::sync::mpsc::channel(capacity);
        self.instances
            .lock()
            .expect("instances poisoned")
            .insert(Participant::host(id, tx.clone()));
        Listener::new((*self.router).clone(), id, tx, rx)
    }

    /// 服务侧句柄（async 门）：给 driver/服务 actor 收发用。
    pub fn handle_for(&self, self_id: InstanceId) -> ServiceHandle {
        ServiceHandle {
            router: self.router.clone(),
            self_addr: Address::instance(self_id),
            supervisor: Arc::clone(&self.supervisor),
        }
    }

    // ---------- 观测面 ----------

    /// 中心路由（观测/测试用）。
    pub fn router(&self) -> &Arc<Router> {
        &self.router
    }

    /// 死信表（ask/tell 失败、超时、晚到回复——可观测，D6）。
    pub fn dead_letters(&self) -> Vec<DeadLetterRecord> {
        self.router.dead_letters()
    }

    /// 消息追踪（一问一答全程留痕，D6）。
    pub fn trace_entries(&self) -> Vec<TraceEntry> {
        self.router.trace_entries()
    }

    /// 未销账的 ask 数（测试/观测）。
    pub fn pending_count(&self) -> usize {
        self.router.pending_count()
    }

    /// 中间层后台任务（driver/测试响应者；crate 内部，不对外暴露 tokio 类型）。
    pub(crate) fn spawn_bg<F>(&self, fut: F) -> tokio::task::JoinHandle<F::Output>
    where
        F: std::future::Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.runtime.spawn(fut)
    }
}

/// 门面级错误（装载语义在 task 3.3 扩展）。
#[derive(Debug, PartialEq, Eq)]
pub enum ServiceError {
    /// 内部服务重复注册。
    DuplicateRegistration(ServiceName),
}

impl std::fmt::Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServiceError::DuplicateRegistration(n) => {
                write!(f, "service already registered: {n}")
            }
        }
    }
}

impl std::error::Error for ServiceError {}

/// 服务侧句柄（**async 门**，D9）：服务 actor/driver 用它发 ask/tell/回复。
/// 不含任何 tokio/kameo 类型（克隆随身携带）。
#[derive(Clone)]
pub struct ServiceHandle {
    router: Arc<Router>,
    self_addr: Address,
    supervisor: Arc<Supervisor>,
}

impl ServiceHandle {
    /// 本服务的地址（作 `from` / 回复方）。
    pub fn address(&self) -> &Address {
        &self.self_addr
    }

    /// 本服务的实例 id。
    pub fn instance_id(&self) -> InstanceId {
        self.self_addr.instance_id()
    }

    /// ask（默认超时，急切分发，executor-agnostic future）。
    pub fn call(
        &self,
        env: Envelope,
    ) -> impl Future<Output = Result<Payload, RouteError>> + Send + 'static {
        self.call_with_timeout(env, self.router.default_timeout())
    }

    /// ask（显式超时）。
    pub fn call_with_timeout(
        &self,
        env: Envelope,
        timeout: Duration,
    ) -> impl Future<Output = Result<Payload, RouteError>> + Send + 'static {
        self.router
            .call_future(env, self.self_addr.clone(), timeout)
    }

    /// tell（无同步反馈，见 [`ServiceManager::tell`] 的约定）。
    pub fn tell(&self, env: Envelope) -> Result<(), RouteError> {
        self.router.send(env)
    }

    /// 回复一个 ask：回复信封带 corr 原路返回 Router 销账（D5/D6：**不按
    /// from 寻址**）。`req` 必须是 ask（无 corr → `NoSuchRoute`）。
    pub fn reply(
        &self,
        req: &Envelope,
        kind: &'static str,
        payload: Payload,
    ) -> Result<(), RouteError> {
        let corr = req.corr.ok_or(RouteError::NoSuchRoute)?;
        let env = Envelope {
            to: req.from.route().clone(),
            from: self.self_addr.clone(),
            corr: Some(corr),
            kind,
            payload,
        };
        self.router.send(env)
    }

    /// 接收方 downcast 失败 → 死信 `TypeMismatch`（spec R1；D5：接收方契约
    /// bug 不是路由错误）。
    pub fn reject(&self, env: &Envelope) -> Result<(), RouteError> {
        self.router.record_reject(env);
        Ok(())
    }

    /// 把后台 future 交宿主 runtime 驱动；panic 按 crash 上报
    /// （kameo actor 生命周期即走此路——插件侧零 tokio TLS，D12 边界发现）。
    pub fn spawn_task(&self, fut: service_core::TaskFuture) {
        let join = self.router.spawn(fut);
        let _ = self.router.spawn(crate::loader::watch_for_crash(
            Arc::clone(&self.supervisor),
            self.self_addr.instance_id(),
            join,
        ));
    }
}

/// [`ServiceHandle`] 即宿主侧 [`Messaging`] 实现：经 `ServiceCtx` 注入服务
/// 构造（core 定义 trait、manager 实现——D11 依赖边的兑现点）。
impl Messaging for ServiceHandle {
    fn instance_id(&self) -> InstanceId {
        self.self_addr.instance_id()
    }

    fn self_address(&self) -> Address {
        self.self_addr.clone()
    }

    fn tell(&self, env: Envelope) -> Result<(), RouteError> {
        ServiceHandle::tell(self, env)
    }

    fn call(&self, env: Envelope) -> service_core::CallFuture {
        Box::pin(self.call_with_timeout(env, self.router.default_timeout()))
    }

    fn reply(
        &self,
        req: &Envelope,
        kind: &'static str,
        payload: Payload,
    ) -> Result<(), RouteError> {
        ServiceHandle::reply(self, req, kind, payload)
    }

    fn reject(&self, env: &Envelope) -> Result<(), RouteError> {
        ServiceHandle::reject(self, env)
    }

    fn spawn_task(&self, fut: service_core::TaskFuture) {
        ServiceHandle::spawn_task(self, fut)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_core::fingerprint;
    use service_core::{ServiceFuture, ServiceState};
    use std::path::PathBuf;
    use tokio::sync::mpsc;

    /// 契约标记（test）：fake 服务的类身份。
    struct FakeContract;

    /// 1.4: 宿主契约表登记与解析（同名同身份幂等、未登记返回 None）。
    ///
    /// 功能：断言 `register_contract` 写入语义与 `resolve_contract` 查询。
    /// 输入：无。
    /// 输出：断言通过。
    #[test]
    fn contract_registry_registers_and_resolves() {
        let dir = temp_root("creg");
        let mgr = ServiceManager::new(&dir).unwrap();
        mgr.register_contract("demo", ContractId::of::<FakeContract>());
        // 同名同身份幂等，不触发断言
        mgr.register_contract("demo", ContractId::of::<FakeContract>());
        assert_eq!(
            mgr.resolve_contract(&"demo".into()),
            Some(ContractId::of::<FakeContract>()),
            "已登记契约名可解析"
        );
        assert_eq!(
            mgr.resolve_contract(&"none".into()),
            None,
            "未登记契约名返回 None（装载期据此显式拒载）"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 1.4: 同名异身份 = 引导期编程错误，断言拦截（design D2）。
    ///
    /// 功能：断言同一契约名重复登记不同身份即 panic。
    /// 输入：无。
    /// 输出：`should_panic` 通过。
    #[test]
    #[should_panic(expected = "同名契约注册必须声明同一身份")]
    fn contract_registry_rejects_conflicting_identity() {
        struct OtherContract;
        let dir = temp_root("creg2");
        let mgr = ServiceManager::new(&dir).unwrap();
        mgr.register_contract("demo", ContractId::of::<FakeContract>());
        mgr.register_contract("demo", ContractId::of::<OtherContract>());
    }

    struct Fake;
    impl service_core::Service for Fake {
        fn start(&mut self) -> Result<(), String> {
            Ok(())
        }
        fn handle(&mut self, _env: service_core::Envelope) -> ServiceFuture<'_> {
            Box::pin(std::future::ready(()))
        }
    }
    fn fake_factory(_ctx: service_core::ServiceCtx) -> Box<dyn service_core::Service> {
        Box::new(Fake)
    }

    struct Other;
    impl service_core::Service for Other {
        fn start(&mut self) -> Result<(), String> {
            Ok(())
        }
        fn handle(&mut self, _env: service_core::Envelope) -> ServiceFuture<'_> {
            Box::pin(std::future::ready(()))
        }
    }
    fn other_factory(_ctx: service_core::ServiceCtx) -> Box<dyn service_core::Service> {
        Box::new(Other)
    }

    fn temp_root(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "smf-mgr-{tag}-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn startup_scan_builds_catalog() {
        let dir = temp_root("startup");
        std::fs::write(dir.join("libalpha.dylib"), b"").unwrap();
        let mgr = ServiceManager::new(&dir).unwrap();
        let names: Vec<_> = mgr.catalog_names().iter().map(|n| n.as_str().to_string()).collect();
        assert_eq!(names, vec!["alpha"], "启动扫描建表");
        match mgr.origin_of(&"alpha".into()).unwrap() {
            ServiceOrigin::Dll(p) => assert_eq!(p, dir.join("libalpha.dylib")),
            other => panic!("origin 不对: {other:?}"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn internal_and_dll_share_one_catalog() {
        let dir = temp_root("shared");
        std::fs::write(dir.join("libbeta.dylib"), b"").unwrap();
        let mgr = ServiceManager::new(&dir).unwrap();
        mgr.register_internal("fake", fake_factory, ContractId::of::<FakeContract>()).unwrap();

        let names: Vec<_> = mgr.catalog_names().iter().map(|n| n.as_str().to_string()).collect();
        assert_eq!(names, vec!["beta", "fake"], "内部与 DLL 同一张表（有序）");
        // 同一访问路径取出
        assert!(matches!(
            mgr.origin_of(&"fake".into()).unwrap(),
            ServiceOrigin::Internal(f) if std::ptr::fn_addr_eq(f, fake_factory as ServiceFactory)
        ));
        // 重复注册被拒
        assert_eq!(
            mgr.register_internal("fake", other_factory, ContractId::of::<FakeContract>()).unwrap_err(),
            ServiceError::DuplicateRegistration("fake".into())
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn reload_replaces_entries_without_touching_instances() {
        let dir = temp_root("reload");
        std::fs::write(dir.join("libgamma.dylib"), b"").unwrap();
        let mgr = ServiceManager::new(&dir).unwrap();

        // "在跑实例": 为 gamma 注册一个实例 (load 语义 3.3 接入, 表先用起来)
        let id = mgr.allocate_id();
        let (tx, _rx) = mpsc::channel(8);
        mgr.instances.lock().unwrap().insert(
            crate::instance::Participant::service_instance(
                id,
                "gamma".into(),
                None,
                ServiceState::Available,
                tx,
            ),
        );

        // 产物被替换: gamma 删除, delta 新增
        std::fs::remove_file(dir.join("libgamma.dylib")).unwrap();
        std::fs::write(dir.join("libdelta.dylib"), b"").unwrap();
        let n = mgr.reload_catalog().unwrap();
        assert_eq!(n, 1, "reload 后只剩 delta");
        assert!(!mgr.has_service(&"gamma".into()), "已删条目从 Catalog 消失");
        assert!(mgr.has_service(&"delta".into()), "新条目出现");

        let instances = mgr.instances.lock().unwrap();
        assert_eq!(instances.service_count(), 1, "在跑实例不受 reload 影响");
        assert!(instances.is_instance_of(id, &"gamma".into()), "实例仍归属原服务");
        drop(instances);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn internal_registrations_survive_reload() {
        let dir = temp_root("internals");
        let mgr = ServiceManager::new(&dir).unwrap();
        mgr.register_internal("fake", fake_factory, ContractId::of::<FakeContract>()).unwrap();
        mgr.reload_catalog().unwrap();
        assert!(mgr.has_service(&"fake".into()), "内部注册跨 reload 保留");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_scan_dir_is_empty_catalog() {
        let dir = std::env::temp_dir().join("smf-mgr-missing-dir");
        let mgr = ServiceManager::new(&dir).unwrap();
        assert!(mgr.catalog_names().is_empty());
    }

    #[test]
    fn host_fingerprint_is_embedded_and_reproducible() {
        // build.rs 固化的宿主指纹非空、带格式前缀
        assert!(
            HOST_FINGERPRINT.starts_with(fingerprint::FINGERPRINT_PREFIX),
            "got {HOST_FINGERPRINT}"
        );
        // 用同一算法重新计算 (rustc 版本 + 契约源树), 必须与固化值一致
        // —— 这正是插件侧 build.rs 将执行的同一计算
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let contract = fingerprint::contract_src_from(manifest).expect("workspace contracts/src");
        let version = fingerprint::rustc_version().expect("rustc --version");
        let recomputed = fingerprint::compute(&version, Some(&contract)).unwrap();
        assert_eq!(
            HOST_FINGERPRINT, recomputed,
            "宿主指纹可重现 ⇒ 与插件侧可比"
        );
        // 一致通过 / 篡改拒载 (报错含 mismatch)
        fingerprint::verify(HOST_FINGERPRINT, HOST_FINGERPRINT).unwrap();
        let err = fingerprint::verify("smf-fp1|tampered", HOST_FINGERPRINT).unwrap_err();
        assert!(err.to_string().contains("mismatch"));
    }
}
