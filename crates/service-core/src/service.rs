//! 服务端点契约：loader 与插件脚手架的**唯一共享点**（design D11 依赖边
//! `manager -> core`、`plugin -> core` 下，契约只能放这里）。
//!
//! 每实例一个 `Box<dyn Service>`，由 driver task 在中间层 runtime 上顺序驱动：
//! - [`Service::start`] 在 `loading` 态执行：`Ok` → `available`，`Err` → `error`；
//! - [`Service::handle`] 逐条处理信封；panic 由 driver 的 JoinError 上报为
//!   crash → `error`（supervisor 唯一恢复权威，D7）。

use crate::envelope::Envelope;
use std::future::Future;
use std::pin::Pin;

/// 异步处理 future（dyn 兼容：`async fn` 无法出现在 object-safe trait 上）。
pub type ServiceFuture<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

/// 每实例服务对象（内部 crate 与 DLL 服务同一契约，D1：仅符号来源不同）。
pub trait Service: Send + Sync + 'static {
    /// `loading` 期初始化；`Err` → `error`（D7）。
    fn start(&mut self) -> Result<(), String>;

    /// 处理一条信封（driver 在 runtime 线程顺序 await，天然背压）。
    fn handle(&mut self, env: Envelope) -> ServiceFuture<'_>;
}

/// 内部服务工厂（`ServiceOrigin::Internal`：编译期函数指针）。
pub type ServiceFactory = fn() -> Box<dyn Service>;

/// DLL 服务工厂（`ServiceOrigin::Dll`：dlopen 取 [`CREATE_SYMBOL`] 符号）。
///
/// # Safety
/// 跨 cdylib 边界返回 `Box<dyn Service>` 依赖"同次编译 + 构建指纹校验"
/// （D12 路线甲）；vtable/代码位于插件内，与"永不 dlclose"（D8）绑定。
pub type DllServiceFactory = unsafe extern "C" fn() -> Box<dyn Service>;

/// 插件必须导出的工厂符号名。
pub const CREATE_SYMBOL: &[u8] = b"service_create";
/// 插件必须导出的构建指纹符号名（`static &'static [u8]`，D8 指纹校验）。
pub const FINGERPRINT_SYMBOL: &[u8] = b"SERVICE_CONTRACT_FINGERPRINT";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Address, InstanceId, Payload, Route};

    struct Echo {
        seen: Vec<String>,
    }

    impl Service for Echo {
        fn start(&mut self) -> Result<(), String> {
            Ok(())
        }

        fn handle(&mut self, env: Envelope) -> ServiceFuture<'_> {
            let kind = env.kind.to_string();
            self.seen.push(kind);
            Box::pin(std::future::ready(()))
        }
    }

    #[test]
    fn factory_creates_boxed_service() {
        let factory: ServiceFactory = || Box::new(Echo { seen: vec![] });
        let mut svc = factory();
        svc.start().expect("start 应成功");
        let env = Envelope::new(
            Route::Instance(InstanceId(1)),
            Address::instance(InstanceId(0)),
            "ping",
            Payload::new(1u32),
        );
        futures::executor::block_on(svc.handle(env));
        // 内部可变性走实现方自选; 这里验证 dyn 分发与 future 可被非 tokio executor await
    }

    #[test]
    fn abi_symbols_are_stable_contract_strings() {
        assert_eq!(CREATE_SYMBOL, b"service_create");
        assert_eq!(FINGERPRINT_SYMBOL, b"SERVICE_CONTRACT_FINGERPRINT");
    }
}
