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
use crate::supervisor::{Action, TransitionError};
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

    /// 装载一个服务实例（多实例 = 多次调用；DLL 走 1 dlopen + N create）。
    ///
    /// 成功返回实例 id；实例进入 `loading`，init 结果经状态事件异步推送
    /// （`Ok` → `available`，`Err` → `error`，spec L5）。
    pub fn load(&self, name: &ServiceName) -> Result<InstanceId, LoadError> {
        let desc = self
            .catalog_entry(name)
            .ok_or_else(|| LoadError::NoSuchService(name.clone()))?;

        // 创建（失败时实例表零写入 — 指纹拒载断言靠这里）
        let id = self.allocate_id();
        let (service, library) = self.create_service(&desc.origin, id, name.clone())?;

        // 插入实例表 (loading) — 状态机从这里起步 (spec L5)
        let (tx, rx) = tokio::sync::mpsc::channel(INSTANCE_OUTLET_BOUND);
        {
            let mut table = self.instances.lock().expect("instances poisoned");
            table.insert(Participant::service_instance(
                id,
                name.clone(),
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

        // monitor: 观察 driver 出口
        let supervisor = Arc::clone(&self.supervisor);
        let _ = self.spawn_bg(async move {
            match join.await {
                Ok(DriverExit::Stopped) | Ok(DriverExit::Closed) | Ok(DriverExit::InitFailed) => {
                    // 主动停用/关闭/init失败: 状态已由相应方设置, 不重复上报
                }
                Err(join_err) if join_err.is_panic() => {
                    // crash → 只上报, **不自动拉起** (D7): 重建只能显式 retry/enable
                    let state = supervisor.state_of(id);
                    let action = match state {
                        Some(ServiceState::Loading) => Action::InitErr, // start/handle 期 panic 未过 loading
                        _ => Action::Crash,
                    };
                    let _ = supervisor.apply(id, action);
                }
                Err(_) => {} // runtime shutdown 取消
            }
        });
        ctrl_tx
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

// 测试在 flow_tests.rs (task 3.3/5.2 端到端: 装载/多实例/unload/指纹)。
