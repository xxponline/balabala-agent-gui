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
    Address, Envelope, InstanceId, Payload, RouteError, ServiceFactory, ServiceName,
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
    /// 中间层自建自持运行时（D9：对外不泄漏 tokio）。
    /// loader（task 3.3）接入前仅构造期使用，属预期。
    #[allow(dead_code)]
    runtime: LayerRuntime,
    /// 中心路由（全部消息过 Router，D3）。
    router: Arc<Router>,
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
        };
        mgr.rebuild_catalog()?;
        Ok(mgr)
    }

    /// 内部服务显式注册：**立即**进 Catalog（与 DLL 条目同表，D8），
    /// 并在每次 reload 时重新并入（程序注册优先于同名 DLL 产物）。
    pub fn register_internal(
        &self,
        name: impl Into<ServiceName>,
        factory: ServiceFactory,
    ) -> Result<(), ServiceError> {
        let desc = ServiceDescriptor::internal(name, factory);
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
    /// loader（task 3.3）接入 driver 后为常规路径。
    #[allow(dead_code)]
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_core::fingerprint;
    use service_core::{ServiceFuture, ServiceState};
    use std::path::PathBuf;
    use tokio::sync::mpsc;

    struct Fake;
    impl service_core::Service for Fake {
        fn start(&mut self) -> Result<(), String> {
            Ok(())
        }
        fn handle(&mut self, _env: service_core::Envelope) -> ServiceFuture<'_> {
            Box::pin(std::future::ready(()))
        }
    }
    fn fake_factory() -> Box<dyn service_core::Service> {
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
    fn other_factory() -> Box<dyn service_core::Service> {
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
        mgr.register_internal("fake", fake_factory).unwrap();

        let names: Vec<_> = mgr.catalog_names().iter().map(|n| n.as_str().to_string()).collect();
        assert_eq!(names, vec!["beta", "fake"], "内部与 DLL 同一张表（有序）");
        // 同一访问路径取出
        assert!(matches!(
            mgr.origin_of(&"fake".into()).unwrap(),
            ServiceOrigin::Internal(f) if std::ptr::fn_addr_eq(f, fake_factory as ServiceFactory)
        ));
        // 重复注册被拒
        assert_eq!(
            mgr.register_internal("fake", other_factory).unwrap_err(),
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
        mgr.register_internal("fake", fake_factory).unwrap();
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
