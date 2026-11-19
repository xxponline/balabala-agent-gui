//! agent-service — 进程内会话薄服务 + AgentTerminal 渲染层（change: `agent-service`）。
//!
//! 设计与消息契约见 change design.md（D1 进程内服务 / D2 薄服务切分 /
//! D3 消息契约 / D7 依赖边）。本 crate **不依赖 service-manager**：
//! 服务经 `service_core::Service` 契约出厂（`ServiceCtx` 自带出站
//! [`service_core::Messaging`] 与 runtime token），GUI 才是 `Listener` 的持有者。
//!
//! 两半结构（随任务推进落地）：
//! - [`service`]：`AgentService` —— 会话注册表、控制/输入 handler、watch 握手、
//!   帧全推与投影事件；
//! - `terminal`（task 4.1 迁入）：`AgentTerminal` —— vt100 parser × N、网格渲染。

pub mod service;

pub use service::{agent_service_factory, AgentService};

#[cfg(test)]
pub(crate) mod testkit;
