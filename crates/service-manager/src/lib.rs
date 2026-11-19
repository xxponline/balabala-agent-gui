//! service-manager — 服务管理框架中间层（Router/Scanner/Loader/Supervisor/自建运行时）。
//!
//! 依赖约束（design D11）：**只依赖 service-core**，永不到 contracts/GUI。
//!
//! 分层：
//! - [`catalog`] / [`scanner`]：启动静态扫描 + 显式 reload 的条目表（D8）；
//! - [`instance`]：实例/参与者表——与 Catalog 结构分离，reload 不触碰（D8）；
//! - [`manager`]：宿主门面（**不泄漏 tokio/kameo 类型**，D9）；
//! - supervisor / router / listener / loader：task 3.4 / 4.2–4.4 / 3.3 接入。
//!
//! 对外 `pub use service_core::*`：宿主侧只经本 crate 引用框架词汇（D11 边）。

pub mod catalog;
pub mod instance;
pub mod manager;
pub mod scanner;

pub use catalog::{Catalog, ServiceDescriptor, ServiceMeta, ServiceOrigin};
pub use instance::{Instances, Participant};
pub use manager::{ServiceError, ServiceManager};
pub use scanner::{scan, ScanError, PLUGIN_EXTENSIONS};

pub use service_core::*;
