//! Loader（task 3.3 / spec `service-lifecycle`：装载与多实例）。
//!
//! - `load()` 流程按 origin 分支（**唯一区别是符号从哪来**，D8）：内部 =
//!   编译期工厂；DLL = dlopen（**每路径 1 次 + N 次创建**，`Arc` 共享）→
//!   **指纹校验（不匹配显式拒载，实例表零写入）** → 取 `service_create`。
//! - 每实例 driver task 在 layer runtime 上顺序驱动 `Service`；init 结果与
//!   crash 经 supervisor 落状态（**kameo/驱动自动 restart 不存在**，D7）。
//! - `unload()` = 停用 + 移出实例表，**不 dlclose**（`ManuallyDrop` 永驻，
//!   真回收 = 进程重启，D8）——API 即此语义，不叫"卸载回收"。
//! - **tell 无同步反馈；GUI 用户操作一律走 ask**（D6，本文件的操作方法同理：
//!   `disable/enable/retry/unload` 返回 `Result` 即同步反馈）。

use crate::instance::Participant;
use crate::manager::{ServiceManager, HOST_FINGERPRINT};
use crate::supervisor::{Action, Supervisor, TransitionError};
use service_core::{
    fingerprint, DllServiceFactory, InstanceId, Service, ServiceCtx, ServiceName, ServiceState,
    CREATE_SYMBOL, FINGERPRINT_SYMBOL,
};
use std::path::PathBuf;
use std::mem::ManuallyDrop;
use std::sync::Arc;
use tokio::sync::watch;

/// 实例 per-outlet 有界通道容量（D6：满即刻失败——容量就是失败边界）。
pub const INSTANCE_OUTLET_BOUND: usize = 64;

/// 装载错误（**创建期**失败；init 异步落状态，不在此列）。
#[derive(Debug)]
pub enum LoadError {
    /// Catalog 门：条目不存在（reload 后被删的条目即此，spec L2）。
    NoSuchService(ServiceName),
    /// 指纹不匹配：显式拒载，**不创建实例**（spec L3；Display 含 mismatch）。
    FingerprintMismatch(fingerprint::FingerprintError),
    /// dlopen 失败。
    Dlopen { path: PathBuf, detail: String },
    /// 导出符号缺失（指纹符号/工厂符号）。
    MissingSymbol { path: PathBuf, symbol: &'static str },
    /// 指纹符号不可读（非 UTF-8 等）。
    BadFingerprint { path: PathBuf, detail: String },
    /// DLL 契约名不在宿主契约表（change `singleton-per-contract` design D2：
    /// 显式拒载，绝不静默）。
    UnknownContract { name: String, path: PathBuf },
    /// 契约名符号不可读（非 UTF-8 等，镜像 `BadFingerprint`）。
    BadContractName { path: PathBuf, detail: String },
    /// 同契约已有实例（每契约恰一实例, spec Single-instance loading per
    /// contract）：同名二次与异名同契约同路径拒载；携带占用方/请求方名字。
    ContractAlreadyLoaded { loaded_name: ServiceName, requested_name: ServiceName },
    /// 工厂 panic（`C-unwind` 穿回宿主被接住——插件工厂不应 panic，脚手架文档有约定）。
    FactoryPanic,
    /// 工厂返回前的宿主侧错误（如服务名参数非法）。
    CreateFailed(String),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::NoSuchService(n) => write!(f, "load: no such service in catalog: {n}"),
            LoadError::FingerprintMismatch(e) => write!(f, "load: {e}"),
            LoadError::Dlopen { path, detail } => {
                write!(f, "load: dlopen failed for {}: {detail}", path.display())
            }
            LoadError::MissingSymbol { path, symbol } => {
                write!(f, "load: missing symbol {symbol} in {}", path.display())
            }
            LoadError::BadFingerprint { path, detail } => {
                write!(f, "load: bad fingerprint symbol in {}: {detail}", path.display())
            }
            LoadError::UnknownContract { name, path } => write!(
                f,
                "load: contract '{name}' unknown to host contract table ({})",
                path.display()
            ),
            LoadError::BadContractName { path, detail } => {
                write!(f, "load: bad contract name symbol in {}: {detail}", path.display())
            }
            LoadError::ContractAlreadyLoaded { loaded_name, requested_name } => write!(
                f,
                "load: contract already loaded by '{loaded_name}'; '{requested_name}' refused \
                 (one instance per contract)"
            ),
            LoadError::FactoryPanic => write!(f, "load: service factory panicked"),
            LoadError::CreateFailed(what) => write!(f, "load: create failed: {what}"),
        }
    }
}

impl std::error::Error for LoadError {}

/// 当前 driver 的控制柄（每个实例至多一个在跑）。
pub(crate) struct DriverControl {
    ctrl: watch::Sender<bool>,
}

/// driver task 出口语义（monitor 依此决定是否上报状态）。
enum DriverExit {
    /// disable/retry/unload 主动停用（状态已由操作方设置）。
    Stopped,
    /// outlet 关闭（参与者被移除）。
    Closed,
    /// init 失败（状态已在 driver 内落 error；原因 v1 不持久化，事件面已够）。
    InitFailed,
}

impl ServiceManager {
    // ---------- 装载 ----------

    /// 装载服务实例：**每契约恰一实例**（change singleton-per-contract,
    /// spec Single-instance loading per contract）——同名二次装载与同契约
    /// 异名装载均显式拒载（占用与状态无关，含 `error`），拒载时零写入。
    ///
    /// 成功返回实例 id；实例进入 `loading`，init 结果经状态事件异步推送
    /// （`Ok` → `available`，`Err` → `error`，spec L5）。
    pub fn load(&self, name: &ServiceName) -> Result<InstanceId, LoadError> {
        let desc = self
            .catalog_entry(name)
            .ok_or_else(|| LoadError::NoSuchService(name.clone()))?;

        // 契约解析（change singleton-per-contract, design D2）：指纹先行
        // （ABI 兼容优先于类判定）→ 读契约名符号查宿主契约表——未知名
        // 显式拒载，实例表零写入；库句柄先入缓存，create 走命中不重开。
        let contract = match &desc.origin {
            crate::catalog::ServiceOrigin::Internal(_) => desc
                .contract
                .expect("内部条目注册即携带契约身份（register_internal, design D1）"),
            crate::catalog::ServiceOrigin::Dll(path) => {
                let lib = self.library_for(path)?;
                let plugin_fp = unsafe { read_fingerprint(&lib, path) }?;
                fingerprint::verify(&plugin_fp, HOST_FINGERPRINT)
                    .map_err(LoadError::FingerprintMismatch)?;
                let cname = unsafe { read_contract_name(&lib, path) }?;
                self.resolve_contract(&ServiceName::from(cname.clone())).ok_or_else(|| {
                    LoadError::UnknownContract { name: cname, path: path.clone() }
                })?
            }
        };

        // 拒载门（spec: Single-instance loading per contract）：契约在表即拒
        // ——占用与状态无关（含 error）；同名二次与异名同契约同一路径；
        // 在 allocate/创建之前返回 ⇒ 实例表零写入、id 不消耗。
        {
            let table = self.instances.lock().expect("instances poisoned");
            if let Some(occupied) = table.contract_instance(contract) {
                let loaded_name = table
                    .get(occupied)
                    .and_then(|p| p.service.clone())
                    .unwrap_or_else(|| ServiceName::from("<unknown>"));
                return Err(LoadError::ContractAlreadyLoaded {
                    loaded_name,
                    requested_name: name.clone(),
                });
            }
        }

        // 创建（失败时实例表零写入 — 指纹/契约拒载断言靠这里）
        let id = self.allocate_id();
        let (service, library) = self.create_service(&desc.origin, id, name.clone())?;

        // 插入实例表 (loading) — 状态机从这里起步 (spec L5)
        let (tx, rx) = tokio::sync::mpsc::channel(INSTANCE_OUTLET_BOUND);
        {
            let mut table = self.instances.lock().expect("instances poisoned");
            table.insert(Participant::service_instance(
                id,
                name.clone(),
                Some(contract),
                ServiceState::Loading,
                tx,
            ));
        }

        let _ = &library; // 库句柄随 libs 缓存常驻 (D8 不 dlclose)
        let ctrl = self.spawn_driver(id, service, rx);
        self.drivers
            .lock()
            .expect("drivers poisoned")
            .insert(id, DriverControl { ctrl });
        Ok(id)
    }

    /// 运行时句柄的不透明载体（插件脚手架按裸指针取 `Handle` 并 enter；
    /// 为何不用 TypeId 见 `service_core::RuntimeToken` 文档）。
    pub(crate) fn runtime_token(&self) -> Arc<dyn service_core::RuntimeToken> {
        Arc::new(HostHandle(self.runtime.handle().clone()))
    }

    /// 按 origin 创建服务对象（内部 vs DLL 唯一差异点，D8）。
    /// 返回 `(服务, Some((库路径, Arc<Library>)))`。
    fn create_service(
        &self,
        origin: &crate::catalog::ServiceOrigin,
        id: InstanceId,
        service: ServiceName,
    ) -> Result<(Box<dyn Service>, Option<(PathBuf, Arc<ManuallyDrop<libloading::Library>>)>), LoadError>
    {
        let messaging: Arc<dyn service_core::Messaging> =
            Arc::new(self.handle_for(id));
        let ctx = ServiceCtx {
            instance: id,
            service,
            messaging,
            runtime_token: self.runtime_token(),
        };
        match origin {
            crate::catalog::ServiceOrigin::Internal(factory) => {
                let svc = catch_panic(|| factory(ctx))?;
                Ok((svc, None))
            }
            crate::catalog::ServiceOrigin::Dll(path) => {
                let (svc, lib) = self.create_dll(path, ctx)?;
                Ok((svc, Some((path.clone(), lib))))
            }
        }
    }

    /// DLL 创建通路（load 与 enable/retry 共用）：dlopen（缓存）→ **指纹
    /// 校验（不匹配拒载）** → 取工厂 → 创建（panic → FactoryPanic）。
    fn create_dll(
        &self,
        path: &std::path::Path,
        ctx: ServiceCtx,
    ) -> Result<(Box<dyn Service>, Arc<ManuallyDrop<libloading::Library>>), LoadError> {
        let lib = self.library_for(path)?;

        // --- 指纹校验 (D8/D12 路线甲): 不匹配显式拒载, 实例表零写入 ---
        let plugin_fp = unsafe { read_fingerprint(&lib, path) }?;
        fingerprint::verify(&plugin_fp, HOST_FINGERPRINT)
            .map_err(LoadError::FingerprintMismatch)?;

        let factory: DllServiceFactory = unsafe {
            *lib.get(CREATE_SYMBOL).map_err(|_| LoadError::MissingSymbol {
                path: path.to_path_buf(),
                symbol: "service_create",
            })?
        };
        let svc = catch_panic(|| unsafe { factory(ctx) })?;
        Ok((svc, lib))
    }

    /// dlopen（每路径一次，`Arc` 共享 = 1 dlopen + N create；缓存**不驱逐**：
    /// `ManuallyDrop` ⇒ 永不 dlclose，真回收 = 进程重启，D8）。
    fn library_for(
        &self,
        path: &std::path::Path,
    ) -> Result<Arc<ManuallyDrop<libloading::Library>>, LoadError> {
        if let Some(lib) = self.libs.lock().expect("libs poisoned").get(path) {
            return Ok(Arc::clone(lib));
        }
        let raw = unsafe { libloading::Library::new(path) }.map_err(|e| LoadError::Dlopen {
            path: path.to_path_buf(),
            detail: e.to_string(),
        })?;
        let lib = Arc::new(ManuallyDrop::new(raw));
        self.libs
            .lock()
            .expect("libs poisoned")
            .insert(path.to_path_buf(), Arc::clone(&lib));
        Ok(lib)
    }

    // ---------- 停用 / 启用 / 重试 / 卸载 / 删除 ----------

    /// 停用（`available` → `offline`，事件先行）：Router 即刻 fail-fast，
    /// driver 随后停（缓冲中的既有件按旧代处理——它们本就 accepted 于 available 期）。
    pub fn disable(&self, id: InstanceId) -> Result<(), TransitionError> {
        self.supervisor.apply(id, Action::Disable)?;
        self.stop_driver(id);
        Ok(())
    }

    /// 启用（`offline` → `loading`）：重建服务对象 + 新 driver；init 结果异步落状态。
    pub fn enable(&self, id: InstanceId) -> Result<(), LoadError> {
        self.respawn(id, Action::Enable)
    }

    /// 重试（`error` → `loading`）：同上（error 态必无在跑 driver，D7）。
    pub fn retry(&self, id: InstanceId) -> Result<(), LoadError> {
        self.respawn(id, Action::Retry)
    }

    fn respawn(&self, id: InstanceId, action: Action) -> Result<(), LoadError> {
        // 状态先行 (offline/error → loading): Router 期间拒绝投递
        self.supervisor
            .apply(id, action)
            .map_err(|_| LoadError::CreateFailed("illegal transition".into()))?;
        let name = {
            let table = self.instances.lock().expect("instances poisoned");
            let p = table.get(id).ok_or_else(|| {
                LoadError::CreateFailed("instance vanished".into())
            })?;
            p.service.clone().expect("service instance")
        };
        let origin = self
            .origin_of(&name)
            .ok_or_else(|| LoadError::NoSuchService(name.clone()))?;
        // 建新服务对象 (loading 中)
        let messaging: Arc<dyn service_core::Messaging> = Arc::new(self.handle_for(id));
        let ctx = ServiceCtx {
            instance: id,
            service: name,
            messaging,
            runtime_token: self.runtime_token(),
        };
        let created = match &origin {
            crate::catalog::ServiceOrigin::Internal(factory) => {
                catch_panic(|| factory(ctx)).map(|s| (s, None))
            }
            crate::catalog::ServiceOrigin::Dll(path) => self
                .create_dll(path, ctx)
                .map(|(s, lib)| (s, Some((path.clone(), lib)))),
        };
        // 任何创建失败都要有终态 (stuck loading 是不可接受的中间态)
        let (service, library) = created.map_err(|e| {
            let _ = self.supervisor.apply(id, Action::InitErr);
            e
        })?;

        let (tx, rx) = tokio::sync::mpsc::channel(INSTANCE_OUTLET_BOUND);
        {
            let mut table = self.instances.lock().expect("instances poisoned");
            if let Some(p) = table.get_mut(id) {
                p.outlet = tx; // 新 driver 的新 outlet
            }
        }
        let _ = &library; // 库句柄随 libs 缓存常驻 (D8 不 dlclose)
        let ctrl = self.spawn_driver(id, service, rx);
        self.drivers
            .lock()
            .expect("drivers poisoned")
            .insert(id, DriverControl { ctrl });
        Ok(())
    }

    /// 卸载（spec L4）：停 driver + 移出实例表；**不 dlclose**（库缓存不驱逐）。
    /// 与 `delete` 不同：unload 不限状态（"停用并移除"），无状态翻转事件
    /// （"不存在"不是状态，D7 口径）。
    pub fn unload(&self, id: InstanceId) -> Result<(), TransitionError> {
        self.stop_driver(id);
        let removed = self
            .instances
            .lock()
            .expect("instances poisoned")
            .remove(id)
            .filter(|p| p.is_service_instance());
        if removed.is_none() {
            return Err(TransitionError::NoSuchInstance(id));
        }
        Ok(())
    }

    /// 删实例（`error` 才可删，spec L5；先停 driver 再走状态机校验）。
    pub fn delete(&self, id: InstanceId) -> Result<(), TransitionError> {
        self.supervisor.delete(id)?;
        self.stop_driver(id);
        Ok(())
    }

    /// 已装载实例的 DLL 库缓存路径数（观测；= 发生过的 dlopen 路径数）。
    pub fn loaded_library_paths(&self) -> Vec<PathBuf> {
        let mut paths: Vec<_> = self
            .libs
            .lock()
            .expect("libs poisoned")
            .keys()
            .cloned()
            .collect();
        paths.sort();
        paths
    }

    // ---------- driver 机制 ----------

    fn stop_driver(&self, id: InstanceId) {
        if let Some(ctrl) = self.drivers.lock().expect("drivers poisoned").get(&id) {
            let _ = ctrl.ctrl.send(true);
        }
    }

    /// 起 driver + monitor（monitor 是 **crash 只上报不拉起**的落点，D7）。
    fn spawn_driver(
        &self,
        id: InstanceId,
        mut service: Box<dyn Service>,
        mut rx: tokio::sync::mpsc::Receiver<service_core::Envelope>,
    ) -> watch::Sender<bool> {
        let (ctrl_tx, mut ctrl_rx) = watch::channel(false);
        let supervisor = Arc::clone(&self.supervisor);

        let join = self.spawn_bg(async move {
            // loading → available/error (spec L5: init 结果落状态)
            if service.start().is_err() {
                let _ = supervisor.apply(id, Action::InitErr);
                return DriverExit::InitFailed;
            }
            let _ = supervisor.apply(id, Action::InitOk);
            loop {
                tokio::select! {
                    env = rx.recv() => match env {
                        Some(env) => service.handle(env).await, // panic → JoinErr → monitor
                        None => return DriverExit::Closed,
                    },
                    changed = ctrl_rx.changed() => {
                        if changed.is_err() || *ctrl_rx.borrow() {
                            return DriverExit::Stopped;
                        }
                    }
                }
            }
        });

        // monitor: 观察 driver 出口 (crash 只上报不拉起, D7)
        let _ = self.spawn_bg(watch_for_crash(
            Arc::clone(&self.supervisor),
            id,
            join,
        ));
        ctrl_tx
    }
}

/// 观察 task 出口：panic → 按 crash 上报（`loading` 期按 init 失败落
/// `error`）——**只上报，不自动拉起**，重建只能显式 retry/enable（D7）。
/// driver 与 kameo actor（`Messaging::spawn_task`）共用此路。
pub(crate) async fn watch_for_crash<T: Send + 'static>(
    supervisor: Arc<Supervisor>,
    id: InstanceId,
    join: tokio::task::JoinHandle<T>,
) {
    if let Err(join_err) = join.await {
        if join_err.is_panic() {
            let state = supervisor.state_of(id);
            let action = match state {
                Some(ServiceState::Loading) => Action::InitErr,
                _ => Action::Crash,
            };
            let _ = supervisor.apply(id, action);
        }
        // 其余 JoinError (runtime shutdown 取消) 不动状态
    }
}

/// 宿主运行时句柄载体（[`service_core::RuntimeToken`] 的宿主实现）。
/// `handle_ptr` 指向内部 `Handle` 字段——插件侧按同版本同 rustc 的
/// `tokio::runtime::Handle` 布局解引用（一起编译前提，D12）。
struct HostHandle(tokio::runtime::Handle);

impl service_core::RuntimeToken for HostHandle {
    fn handle_ptr(&self) -> *const () {
        &self.0 as *const tokio::runtime::Handle as *const ()
    }
}

/// catch 插件工厂 panic（`C-unwind` 允许穿回；宿主侧接住 → `FactoryPanic`）。
fn catch_panic(
    f: impl FnOnce() -> Box<dyn Service>,
) -> Result<Box<dyn Service>, LoadError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
        .map_err(|_| LoadError::FactoryPanic)
}

/// 读取插件导出的指纹符号（`static SERVICE_CONTRACT_FINGERPRINT: &[u8]`）。
///
/// # Safety
/// `lib` 必须是有效库句柄（调用方经 `library_for` 保证）。
unsafe fn read_fingerprint(
    lib: &ManuallyDrop<libloading::Library>,
    path: &std::path::Path,
) -> Result<String, LoadError> {
    let sym: libloading::Symbol<'_, *const &[u8]> = unsafe {
        lib.get(FINGERPRINT_SYMBOL)
    }
    .map_err(|_| LoadError::MissingSymbol {
        path: path.to_path_buf(),
        symbol: std::str::from_utf8(FINGERPRINT_SYMBOL).unwrap_or("?"),
    })?;
    let bytes: &[u8] = unsafe { **sym };
    std::str::from_utf8(bytes)
        .map(|s| s.to_string())
        .map_err(|e| LoadError::BadFingerprint {
            path: path.to_path_buf(),
            detail: e.to_string(),
        })
}

/// 读取插件导出的契约名符号（`static SERVICE_CONTRACT_NAME: &[u8]`）。
///
/// # Safety
/// `lib` 必须是有效库句柄（调用方经 `library_for` 保证）。
unsafe fn read_contract_name(
    lib: &ManuallyDrop<libloading::Library>,
    path: &std::path::Path,
) -> Result<String, LoadError> {
    let sym: libloading::Symbol<'_, *const &[u8]> = unsafe {
        lib.get(service_core::CONTRACT_NAME_SYMBOL)
    }
    .map_err(|_| LoadError::MissingSymbol {
        path: path.to_path_buf(),
        symbol: std::str::from_utf8(service_core::CONTRACT_NAME_SYMBOL).unwrap_or("?"),
    })?;
    let bytes: &[u8] = unsafe { **sym };
    std::str::from_utf8(bytes)
        .map(|s| s.to_string())
        .map_err(|e| LoadError::BadContractName {
            path: path.to_path_buf(),
            detail: e.to_string(),
        })
}

// 测试在 flow_tests.rs (task 3.3/5.2 端到端: 装载/多实例/unload/指纹)。
