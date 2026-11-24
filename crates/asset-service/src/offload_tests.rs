//! task 3.3 的验证：**枚举的同步 fs 必须离开单 worker**（design D4 修订）。
//!
//! `service-layer` 运行时是 `worker_threads(1)`，`service.handle(env)`
//! 直接在其上 `await`。一旦枚举的 `std::fs::read_dir` 落在这个 worker
//! 上，同层的 agent / solution 服务就全部排队（spec: Enumeration does
//! not delay other services）。
//!
//! 断言方式：用一个**门闩 filter** 在扫描中途卡住枚举，分别记录
//! ① 驱动后台任务的 worker 线程 ② 实际执行 `is_ignored` 的线程，
//! 两者**必须不同**。若哪天有人把 `spawn_blocking` 去掉，fs 就会回到
//! worker 上执行 ⇒ 断言失败。
//!
//! 门闩带**看门狗自动放行**（3s）—— 实现写错时测试是「失败」而不是
//! 「挂死」。

use crate::contract::{
    AssetRegistration, AssetState, EnumerationDone, ModuleRegistration, ProjectRegistration,
    KIND_ENUM_DONE, KIND_REGISTER,
};
use crate::service::{AssetService, asset_service_factory};
use service_core::{
    Address, CallFuture, Envelope, InstanceId, Messaging, Payload, Route, RouteError, RuntimeToken,
    Service, ServiceCtx, TaskFuture,
};
use std::any::Any;
use std::sync::{Arc, Mutex};
use std::time::Duration;

// ---------------- 会真的往 tokio 上 spawn 的 Messaging ----------------

#[derive(Default)]
struct TokioMessaging {
    sent: Mutex<Vec<Envelope>>,
    /// 驱动后台任务的线程（= 单 worker）。
    worker: Arc<Mutex<Option<std::thread::ThreadId>>>,
}

impl Messaging for TokioMessaging {
    fn instance_id(&self) -> InstanceId {
        InstanceId(77)
    }

    fn self_address(&self) -> Address {
        Address::instance(InstanceId(77))
    }

    fn tell(&self, env: Envelope) -> Result<(), RouteError> {
        self.sent.lock().expect("sent poisoned").push(env);
        Ok(())
    }

    fn call(&self, env: Envelope) -> CallFuture {
        self.sent.lock().expect("sent poisoned").push(env);
        Box::pin(std::future::ready(Ok(Payload::new(()))))
    }

    fn reply(
        &self,
        req: &Envelope,
        kind: &'static str,
        payload: Payload,
    ) -> Result<(), RouteError> {
        let corr = req.corr.ok_or(RouteError::NoSuchRoute)?;
        self.sent.lock().expect("sent poisoned").push(Envelope {
            to: req.from.route().clone(),
            from: self.self_address(),
            corr: Some(corr),
            kind,
            payload,
        });
        Ok(())
    }

    fn reject(&self, env: &Envelope) -> Result<(), RouteError> {
        self.sent.lock().expect("sent poisoned").push(env.clone());
        Ok(())
    }

    fn spawn_task(&self, fut: TaskFuture) {
        let worker = self.worker.clone();
        tokio::spawn(async move {
            // 记录「谁在 poll 这个后台任务」——正确实现下它是单 worker。
            *worker.lock().expect("worker poisoned") = Some(std::thread::current().id());
            fut.await;
        });
    }
}

#[derive(Default)]
struct NullRuntime;

impl RuntimeToken for NullRuntime {
    fn handle_ptr(&self) -> *const () {
        std::ptr::null()
    }
}

fn ask(kind: &'static str, payload: impl Any + Send + Sync) -> Envelope {
    Envelope::new(
        Route::Instance(InstanceId(1)),
        Address::instance(InstanceId(1)),
        kind,
        Payload::new(payload),
    )
    .with_corr(service_core::Corr(1))
}

// ---------------- 测试 ----------------

/// 单 worker 上，枚举的同步 fs **不得**跑在 worker 线程（design D4 /
/// spec: Enumeration does not delay other services）。
///
/// 服务把「执行本次枚举的线程」随 `asset.enum_done` 带回（`cfg(test)`
/// 观测面），与「驱动后台任务的 worker 线程」比对。去掉 `spawn_blocking`
/// 会让同步 fs 落回 worker ⇒ 断言失败。
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn enumeration_keeps_the_single_worker_free() {
    // 真实可扫的目录 + 保证 `.gitignore` 读取与 read_dir 都跑起来
    let dir = std::env::temp_dir().join(format!("asset-offload-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(dir.join(".gitignore"), "*.log\n").expect("write");
    std::fs::write(dir.join("main.rs"), "fn main() {}").expect("write");

    let msg = Arc::new(TokioMessaging::default());
    let ctx = ServiceCtx {
        instance: InstanceId(77),
        service: "asset".to_string().into(),
        messaging: msg.clone(),
        runtime_token: Arc::new(NullRuntime),
    };
    let mut svc = AssetService::new(ctx);

    let project = ProjectRegistration {
        name: "p".into(),
        root: dir.clone(),
        is_git: true, // ⇒ 现读 root/.gitignore（同步 fs）
        modules: vec![ModuleRegistration {
            name: "p".into(),
            root: dir.clone(),
            children: Vec::new(),
        }],
    };

    // 注册 ⇒ 交接后台（handle 立即返回，视图仍是清场后的 Empty）
    drop(svc.handle(ask(
        KIND_REGISTER,
        AssetRegistration {
            workspace_dir: dir.clone(),
            projects: vec![project],
        },
    )));
    assert_eq!(
        svc.snapshot().state,
        AssetState::Empty,
        "交接后视图尚未落地（枚举在途）"
    );

    // 等枚举完成并回投（本测试的 Messaging 只捕获出站信封、不路由回服务，
    // 故断言回投载荷；「落地转 Forest」由 service::tests 的单测覆盖）
    let mut done: Option<EnumerationDone> = None;
    for _ in 0..600 {
        done = msg
            .sent
            .lock()
            .expect("sent poisoned")
            .iter()
            .find(|e| e.kind == KIND_ENUM_DONE)
            .and_then(|e| e.payload.downcast::<EnumerationDone>())
            .cloned();
        if done.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let done = done.expect("枚举应完成并回投");
    let nodes = done.result.as_ref().expect("扫描应成功");
    assert_eq!(nodes.len(), 1, "回投携带枚举产物");

    // ---- 核心断言 ----
    let worker = msg
        .worker
        .lock()
        .expect("worker poisoned")
        .expect("后台任务应已被驱动");
    let fs_thread = done.fs_thread.expect("观测面应带回 fs 线程");
    assert_ne!(
        fs_thread, worker,
        "枚举的同步 fs 跑回了单 worker —— 同层服务会被整段 read_dir + .gitignore 读取卡住 (design D4)"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// 工厂产出的实例能 `start()`（task 3.5 的一半在 service 单测里，这里再
/// 用真实 tokio 上下文复核一次）。
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn factory_instance_starts_on_a_real_runtime() {
    let msg = Arc::new(TokioMessaging::default());
    let ctx = ServiceCtx {
        instance: InstanceId(78),
        service: "asset".to_string().into(),
        messaging: msg,
        runtime_token: Arc::new(NullRuntime),
    };
    let mut svc = asset_service_factory(ctx);
    assert!(svc.start().is_ok());
}
