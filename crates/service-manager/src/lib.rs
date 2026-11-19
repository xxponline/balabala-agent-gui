//! service-manager — 服务管理框架中间层（Router/Scanner/Loader/Supervisor/自建运行时）。
//!
//! 依赖约束（design D11）：**只依赖 service-core**，永不到 contracts/GUI。
//!
//! 分层：
//! - [`catalog`] / [`scanner`]：启动静态扫描 + 显式 reload 的条目表（D8）；
//! - [`instance`]：实例/参与者表——与 Catalog 结构分离，reload 不触碰（D8）；
//! - [`supervisor`]：每实例状态机——唯一恢复权威，kameo 自动 restart 禁用（D7）；
//! - [`router`]：中心枢纽——统一信封、两级寻址、ask/超时、fail-fast、死信；
//! - [`listener`]：拉模式 actor 的 mailbox（poll 门，**无回调 API**，D9）；
//! - [`runtime`]：自建自持 tokio（对外不泄漏 tokio，D9）；
//! - [`manager`]：宿主门面（双门 API）；loader → task 3.3 接入。
//!
//! 对外 `pub use service_core::*`：宿主侧只经本 crate 引用框架词汇（D11 边）。
//!
//! ## API 三条承诺（对照 design D6/D8/D9；task 7.1 收口）
//!
//! 1. **`unload()` 是停用，不是卸载回收**（D8）：停用并移出实例表，
//!    **永不 dlclose**（`ManuallyDrop` 永驻缓存）——真回收 = 进程重启；
//!    API 文档/命名不得暗示可回收动态库。
//! 2. **GUI poll 门为第一公民，无回调 API**（D9，B 方案）：宿主参与者
//!    （GUI）= 拉模式 actor，`Listener::{poll,drain,snapshot,submit}` 同步、
//!    调用方线程执行；跨线程回调合同问题从结构上消除；future 门仅供
//!    runtime 驱动的服务/测试。
//! 3. **tell 无同步反馈，用户操作一律走 ask**（D6）：tell 失败只落死信
//!    （观测面）；GUI 用户触发的操作走 ask，错误可回显给用户。

pub mod catalog;
pub mod instance;
pub mod listener;
pub mod loader;
pub mod manager;
pub mod router;
pub mod runtime;
pub mod scanner;
pub mod supervisor;

#[cfg(test)]
mod flow_tests;

pub use catalog::{Catalog, ServiceDescriptor, ServiceMeta, ServiceOrigin};
pub use instance::{Instances, Participant};
pub use listener::Listener;
pub use loader::{LoadError, INSTANCE_OUTLET_BOUND};
pub use manager::{ServiceError, ServiceHandle, ServiceManager, HOST_FINGERPRINT};
pub use router::{DeadLetterRecord, Router, TraceDirection, TraceEntry};
pub use scanner::{scan, ScanError, PLUGIN_EXTENSIONS};
pub use supervisor::{Action, Supervisor, TransitionError};

pub use service_core::*;
