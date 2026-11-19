//! 双向共享消息类型的中立命名空间（design D2）。
//!
//! 归属规则：
//! - **单向消息归调用方**：调用方拥有自己发的消息与期待的回复类型，放自己
//!   服务的命名空间（如 `contracts::agent`、`contracts::asset`），被调方
//!   import 调用方契约；
//! - **双向共享的消息类型归本命名空间**：裂分独立契约 crate 时它 = 独立
//!   中立 crate（零依赖），保证 `contract-a` / `contract-b` 永不形成编译期
//!   互依。
//!
//! 本 crate **零依赖**（`cargo build -p contracts` 不引入任何外部 crate）。
