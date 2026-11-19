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

pub mod catalog;
pub mod instance;
pub mod listener;
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
pub use manager::{ServiceError, ServiceHandle, ServiceManager};
pub use router::{DeadLetterRecord, Router, TraceDirection, TraceEntry};
pub use scanner::{scan, ScanError, PLUGIN_EXTENSIONS};
pub use supervisor::{Action, Supervisor, TransitionError};

pub use service_core::*;
