//! 服务端点契约：loader 与插件脚手架的**唯一共享点**（design D11 依赖边
//! `manager -> core`、`plugin -> core` 下，契约只能放这里）。
//!
//! 每实例一个 `Box<dyn Service>`：
//! - 工厂签名 `fn(ServiceCtx) -> Box<dyn Service>`：内部服务与 DLL 服务
//!   **同一契约**（D1：仅符号来源不同），构造时即拿到自己的身份与出站能力；
//! - [`Service::start`] 在 `loading` 态执行：`Ok` → `available`，`Err` → `error`；
//! - [`Service::handle`] 逐条处理信封；panic 由 driver 的 JoinError 上报为
//!   crash → `error`（supervisor 唯一恢复权威，D7）。

use crate::envelope::{Envelope, Payload};
use crate::error::RouteError;
use crate::route::{Address, InstanceId, ServiceName};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// 异步处理 future（dyn 兼容：`async fn` 无法出现在 object-safe trait 上）。
pub type ServiceFuture<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

/// ask future（executor-agnostic：只剩 oneshot await，D9）。
pub type CallFuture = Pin<Box<dyn Future<Output = Result<Payload, RouteError>> + Send + 'static>>;

/// 交给宿主 runtime 驱动的后台 future（[`Messaging::spawn_task`]）。
pub type TaskFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

/// 服务的出站消息能力：由宿主实现（manager 的 [`ServiceHandle`]），经
/// [`ServiceCtx`] 注入——core 只定义 trait，实现留在 manager（D11 边）。
pub trait Messaging: Send + Sync + 'static {
    /// 本服务实例 id。
    fn instance_id(&self) -> InstanceId;
    /// 本服务地址（作 `from`）。
    fn self_address(&self) -> Address;
    /// tell（无同步反馈，D6）。
    fn tell(&self, env: Envelope) -> Result<(), RouteError>;
    /// ask（默认超时，急切分发，返回可任意 executor await 的 future，D9）。
    fn call(&self, env: Envelope) -> CallFuture;
    /// 回复一个 ask（回复带 corr 走总线销账，D5/D6）。
    fn reply(
        &self,
        req: &Envelope,
        kind: &'static str,
        payload: Payload,
    ) -> Result<(), RouteError>;
    /// 接收方 downcast 失败：拒收并落死信（`TypeMismatch`）——这是接收方
    /// 契约 bug，不是路由错误（D5 / spec R1）。
    fn reject(&self, env: &Envelope) -> Result<(), RouteError>;
    /// 把后台 future 交给**宿主 runtime** 驱动（返回后不阻塞）。
    ///
    /// 跨 dylib 的关键约束（D12 边界发现）：tokio/kameo 的 ambient 上下文
    /// 是**每份二进制副本各自的 thread-local**——插件里调 `tokio::spawn`
    /// 看不到宿主 runtime。因此插件侧一切 spawn 经本方法由宿主代跑；
    /// 宿主实现须在 panic 时按 crash 上报（supervisor 唯一恢复权威，D7）。
    fn spawn_task(&self, fut: TaskFuture);
}

/// 运行时句柄载体：宿主实现、插件消费（core 不依赖 tokio——D9/D11）。
///
/// 为什么**不是** `Any` + TypeId downcast（D12 第三个边界发现）：tokio 依赖
/// proc-macro 链（syn/quote），其 metadata 随 cargo **解析范围**变化（workspace
/// vs `-p` 构建 syn feature 不同）→ tokio 编译两份 → TypeId 分叉 → downcast
/// 静默失败。契约 crate 零依赖、metadata 确定，所以 `Ping`/`Pong` 这类契约
/// 类型跨界 TypeId 稳定；**依赖链可变的类型不得跨边界作 TypeId 用途**。
/// 故走裸指针：两侧同 rustc / 同 tokio 版本 / 同 target（= D12 路线甲的
/// "一起编译"前提）⇒ `tokio::runtime::Handle` 布局一致。
pub trait RuntimeToken: Send + Sync + 'static {
    /// 指向宿主侧 `tokio::runtime::Handle` 的裸指针（不得为悬垂）。
    fn handle_ptr(&self) -> *const ();
}

/// 工厂入参：服务构造时即拿到身份 + 出站能力（driver 持 ctx 驱动本实例）。
#[derive(Clone)]
pub struct ServiceCtx {
    pub instance: InstanceId,
    pub service: ServiceName,
    pub messaging: Arc<dyn Messaging>,
    /// 宿主运行时句柄载体（插件脚手架取 `Handle` 并逐 poll `enter`——
    /// 跨 dylib 时 tokio ambient 上下文是**每份二进制副本各自的 thread-
    /// local**，不 enter 就看不到宿主 runtime）。
    pub runtime_token: Arc<dyn RuntimeToken>,
}

/// 每实例服务对象（内部 crate 与 DLL 服务同一契约，D1：仅符号来源不同）。
pub trait Service: Send + Sync + 'static {
    /// `loading` 期初始化；`Err` → `error`（D7）。
    fn start(&mut self) -> Result<(), String>;

    /// 处理一条信封（driver 在 runtime 线程顺序 await，天然背压）。
    fn handle(&mut self, env: Envelope) -> ServiceFuture<'_>;
}

/// 内部服务工厂（`ServiceOrigin::Internal`：编译期函数指针）。
pub type ServiceFactory = fn(ServiceCtx) -> Box<dyn Service>;

/// DLL 服务工厂（`ServiceOrigin::Dll`：dlopen 取 [`CREATE_SYMBOL`] 符号）。
///
/// 用 `C-unwind` 而非 `C`：插件工厂与宿主同仓同编译（D11 "一起编译"），
/// panic 可以安全地穿回宿主并被 `catch_unwind` 接住（= `LoadError::FactoryPanic`）；
/// `extern "C"` 的 unwind-out 会 abort，故显式选 `C-unwind`。
///
/// # Safety
/// 跨 cdylib 边界返回 `Box<dyn Service>` 依赖"同次编译 + 构建指纹校验"
/// （D12 路线甲）；vtable/代码位于插件内，与"永不 dlclose"（D8）绑定。
/// FFI-safety 警告是有意接受的：两侧同 rustc 同次构建（由 D8 指纹兜底），
/// 胖指针（data + vtable）经 sret 返回在同编译器下形状一致。
#[allow(improper_ctypes_definitions)]
pub type DllServiceFactory = unsafe extern "C-unwind" fn(ServiceCtx) -> Box<dyn Service>;

/// 插件必须导出的工厂符号名。
pub const CREATE_SYMBOL: &[u8] = b"service_create";
/// 插件必须导出的构建指纹符号名（`static &'static [u8]`，D8 指纹校验）。
pub const FINGERPRINT_SYMBOL: &[u8] = b"SERVICE_CONTRACT_FINGERPRINT";
/// 插件必须导出的契约名符号名（`static &'static [u8]`；change
/// `singleton-per-contract` design D2——装载期读出后查宿主契约表解析身份）。
pub const CONTRACT_NAME_SYMBOL: &[u8] = b"SERVICE_CONTRACT_NAME";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::route::Route;

    /// 测试用出站能力：记录发出的信封（不真投递）。
    struct TestMessaging {
        sent: std::sync::Mutex<Vec<Envelope>>,
        id: InstanceId,
    }

    impl Messaging for TestMessaging {
        fn instance_id(&self) -> InstanceId {
            self.id
        }
        fn self_address(&self) -> Address {
            Address::instance(self.id)
        }
        fn tell(&self, env: Envelope) -> Result<(), RouteError> {
            self.sent.lock().unwrap().push(env);
            Ok(())
        }
        fn call(&self, env: Envelope) -> CallFuture {
            self.sent.lock().unwrap().push(env);
            Box::pin(std::future::ready(Ok(Payload::new(1u8))))
        }
        fn reply(
            &self,
            req: &Envelope,
            kind: &'static str,
            payload: Payload,
        ) -> Result<(), RouteError> {
            let corr = req.corr.ok_or(RouteError::NoSuchRoute)?;
            let env = Envelope {
                to: req.from.route().clone(),
                from: Address::instance(self.id),
                corr: Some(corr),
                kind,
                payload,
            };
            self.sent.lock().unwrap().push(env);
            Ok(())
        }

        fn reject(&self, env: &Envelope) -> Result<(), RouteError> {
            self.sent.lock().unwrap().push(env.clone());
            Ok(())
        }

        fn spawn_task(&self, _fut: TaskFuture) {
            // 测试用出站能力不驱动后台任务
        }
    }

    struct Echo {
        ctx: ServiceCtx,
        started: bool,
        seen: Vec<&'static str>,
    }

    impl Service for Echo {
        fn start(&mut self) -> Result<(), String> {
            self.started = true;
            Ok(())
        }

        fn handle(&mut self, env: Envelope) -> ServiceFuture<'_> {
            self.seen.push(env.kind);
            if env.corr.is_some() {
                let _ = self
                    .ctx
                    .messaging
                    .reply(&env, "pong", Payload::new(env.kind.len() as u64));
            }
            Box::pin(std::future::ready(()))
        }
    }

    struct NullToken;
    impl RuntimeToken for NullToken {
        fn handle_ptr(&self) -> *const () {
            std::ptr::null()
        }
    }

    fn test_ctx(id: u64) -> (ServiceCtx, Arc<TestMessaging>) {
        let m = Arc::new(TestMessaging {
            sent: std::sync::Mutex::new(vec![]),
            id: InstanceId(id),
        });
        let ctx = ServiceCtx {
            instance: m.id,
            service: "echo".into(),
            messaging: m.clone(),
            runtime_token: Arc::new(NullToken),
        };
        (ctx, m)
    }

    #[test]
    fn factory_creates_service_with_ctx_and_replies_via_messaging() {
        let factory: ServiceFactory = |ctx| {
            Box::new(Echo {
                ctx,
                started: false,
                seen: vec![],
            })
        };
        let (ctx, m) = test_ctx(1);
        let mut svc = factory(ctx);
        svc.start().expect("start 应成功");

        let ask = Envelope::new(
            Route::Instance(InstanceId(1)),
            Address::instance(InstanceId(9)),
            "ping",
            Payload::new(()),
        )
        .with_corr(crate::route::Corr(5));
        futures::executor::block_on(svc.handle(ask));

        let sent = m.sent.lock().unwrap();
        assert_eq!(sent.len(), 1, "回复经 messaging 出站");
        assert_eq!(sent[0].kind, "pong");
        assert_eq!(sent[0].corr, Some(crate::route::Corr(5)), "回复带 corr 销账");
        assert_eq!(sent[0].to, Route::Instance(InstanceId(9)), "回给调用方");
    }

    #[test]
    fn abi_symbols_are_stable_contract_strings() {
        assert_eq!(CREATE_SYMBOL, b"service_create");
        assert_eq!(FINGERPRINT_SYMBOL, b"SERVICE_CONTRACT_FINGERPRINT");
        assert_eq!(CONTRACT_NAME_SYMBOL, b"SERVICE_CONTRACT_NAME");
    }

    #[test]
    fn call_future_is_plain_future() {
        // executor-agnostic: 任意 block_on 可 await
        let (ctx, _m) = test_ctx(2);
        let fut = ctx.messaging.call(Envelope::new(
            Route::Instance(InstanceId(3)),
            Address::instance(InstanceId(2)),
            "q",
            Payload::new(()),
        ));
        let r = futures::executor::block_on(fut).unwrap();
        assert!(r.downcast::<u8>().is_some());
    }
}
