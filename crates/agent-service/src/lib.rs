//! agent-service — 进程内会话薄服务 + AgentTerminal 渲染层（change: `agent-service`）。
//!
//! 设计与消息契约见 change design.md（D1 进程内服务 / D2 薄服务切分 /
//! D3 消息契约 / D7 依赖边）。本 crate **不依赖 service-manager**：
//! 服务经 `service_core::Service` 契约出厂（`ServiceCtx` 自带出站
//! [`service_core::Messaging`] 与 runtime token），GUI 才是 `Listener` 的持有者。
//!
//! # 消息契约表（design D3）
//!
//! 载荷类型见 [`contract`]；kind 常量同模块。全部经 `Payload::Typed(Arc)`
//! 零序列化透传（信封对 payload 不解释，D2）。
//!
//! | 方向 | kind | 载荷 | 形态 |
//! |---|---|---|---|
//! | GUI→服务 | `select` / `create` / `close` | `SelectReq` / `CreateReq` / `CloseReq` | **ask**（D6 用户操作一律 ask）；回执 kind `control.reply`，载荷 `ControlResult<_>` |
//! | GUI→服务 | `input.key` / `input.text` / `input.wheel` / `input.click` | `InputKey` / `InputText` / `InputWheel` / `InputClick` | **tell**（静默失败，D6） |
//! | GUI→服务 | `resize` / `theme` | `ResizeReq` / `ThemeReq` | tell（服务侧 `(cols,rows)` 去重 / 快照更新 + 广播） |
//! | GUI→服务 | `watch` | `WatchReq` | tell（订阅身份取信封 `from`） |
//! | 服务→GUI | `frame_bytes` | `FrameBytes { session, Arc<Vec<u8>>, prod_gen }` | tell → watcher：全推（仅变字节）、字节所有权移动零拷贝、`prod_gen` = 生产代际（帧代际号原语，任务 1.2）、出口满丢帧计数 |
//! | 服务→GUI | `sessions_changed` | `SessionsChanged(SessionView)` | tell → watcher：事件自带快照（D10 零回程） |
//! | 服务内部 | `drain.tick` | —（service 私有） | 循环自寻址（D5：tick 仍在 driver 顺序上下文） |
//! | 框架既有 | `state_change` / `route_error` | `StateChange` / `RouteError` | change service-management-framework 6.1 |
//!
//! 两半结构（随任务推进落地）：
//! - [`service`]：`AgentService` —— 会话注册表、控制/输入 handler、watch 握手、
//!   帧全推与投影事件；
//! - [`contract`]：GUI ↔ 服务消息契约（design D3）；
//! - [`session`]：`SessionIo` 会话句柄抽象；
//! - [`shell`]：`PiAgentShell` — 真实嵌入会话（生产 spawn，task 3.5 迁入）；


pub mod contract;
pub mod frame_gen;
pub mod service;
pub mod session;
pub mod shell;
pub mod terminal;

pub use service::{agent_service_factory, AgentService};
pub use shell::PiAgentShell;

#[cfg(test)]
pub(crate) mod testkit;
