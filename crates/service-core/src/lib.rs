//! service-core — 服务管理框架的词汇表（中间层三件套之一）。
//!
//! 依赖约束（design D11）：**零上层依赖**——不依赖 contracts、GUI 或具体服务。
//!
//! 内容：
//! - [`route`]：两级寻址 `Route::Instance(..)` / `Route::Service(name, policy)`，
//!   以及 `from` 专用的受限地址 [`Address`]（拒绝歧义的 `Service(..)`）。
//! - [`envelope`]：统一信封 [`Envelope`] 与类型化 payload（`Arc` 透传，零序列化）。
//! - [`state`]：每实例标准状态 [`ServiceState`] 与状态翻转事件 [`StateChange`]。
//! - [`error`]：[`RouteError`]（Timeout / TargetNotAvailable / DeadLetter /
//!   NoSuchRoute / TypeMismatch / AmbiguousAddress）。
//!
//! service-manager 对外 `pub use` 本 crate 的词汇（宿主侧只经 manager 引用），
//! 保持 D11 的依赖边：GUI -> contracts + manager；manager -> core。

pub mod envelope;
pub mod error;
pub mod route;
pub mod service;
pub mod state;

pub use envelope::{Envelope, Payload};
pub use error::{DeadLetterReason, RouteError};
pub use route::{Address, Corr, InstanceId, Route, SelectPolicy, ServiceName};
pub use service::{
    DllServiceFactory, Service, ServiceFactory, ServiceFuture, CREATE_SYMBOL,
    FINGERPRINT_SYMBOL,
};
pub use state::{ServiceState, StateChange};

/// 框架级 `kind`：状态翻转事件（payload = [`StateChange`]）。
pub const KIND_STATE_CHANGE: &str = "state_change";
