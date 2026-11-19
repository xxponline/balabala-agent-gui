//! service-plugin — 插件侧脚手架（中间层三件套之一）。
//!
//! 依赖约束（design D11）：`plugin -> contracts + core`——**不依赖** manager 或
//! GUI。宿主（service-manager loader）与本 crate 共享的唯一契约在
//! [`service_core`]（`Service` 端点契约 + ABI 符号 + 构建指纹算法）。
//!
//! 本 change 落地内容：
//! - [`build`]：构建指纹嵌入模板（task 3.2）——插件 build.rs 调用，
//!   生成 `SERVICE_CONTRACT_FINGERPRINT` 导出符号；
//! - [`actor`]：kameo actor 脚手架（`ActorService` + `Ingress`），
//!   外加 D12 路线乙的 typed extern fn 预留形状（task 5.1）。
//! - extern 入口模板：见 [`actor`] 文档与 `crates/example-plugin` 实例。

pub mod actor;
pub mod build;

pub use actor::{actor_service, ActorService, Ingress, IngressFuture, TypedIngress};
