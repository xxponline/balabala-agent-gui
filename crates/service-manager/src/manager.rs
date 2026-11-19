//! ServiceManager 门面：Catalog/实例表/reload（task 3.1 范围；
//! runtime/Router/Supervisor/Loader 由 task 3.4/4.x/3.3 逐个接入本结构）。

use crate::catalog::{Catalog, ServiceDescriptor, ServiceOrigin};
use crate::instance::Instances;
use crate::scanner::{self, ScanError};
use service_core::{InstanceId, ServiceFactory, ServiceName};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, RwLock};

/// 服务管理中间层门面（对宿主暴露，**不泄漏 tokio/kameo 类型**，D9）。
pub struct ServiceManager {
    /// 服务扫描目录（= plugins/ 落点，D8 静态扫描、无 watch）。
    scan_dir: PathBuf,
    /// 内部服务显式注册（持久，reload 后重新并入，程序注册优先于同名 DLL）。
    internals: Mutex<Vec<ServiceDescriptor>>,
    /// Catalog：唯一被 reload 改写的共享状态。
    catalog: RwLock<Catalog>,
    /// 实例/参与者表：与 Catalog **结构分离**，reload 不触碰（D8）。
    pub(crate) instances: Mutex<Instances>,
    /// 参与者 id 分配（服务实例与宿主参与者共用计数）。
    next_id: AtomicU64,
}

impl ServiceManager {
    /// 创建并执行启动扫描（D8：启动扫一遍建 Catalog）。
    pub fn new(scan_dir: impl Into<PathBuf>) -> Result<Self, ScanError> {
        let mgr = ServiceManager {
            scan_dir: scan_dir.into(),
            internals: Mutex::new(Vec::new()),
            catalog: RwLock::new(Catalog::new()),
            instances: Mutex::new(Instances::new()),
            next_id: AtomicU64::new(1),
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

#[cfg(test)]
mod tests {
    use super::*;
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
}
