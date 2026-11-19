//! contracts — 全部服务 Message 的单一 crate（design D2）。
//!
//! 归属规则：**消息归调用方**（调用方拥有自己发的消息与期待的回复类型，
//! 放自己服务的命名空间），被调方 import 调用方契约；**双向共享的消息类型
//! 归 `contracts::shared` 中立命名空间**（裂分时 = 独立中立 crate，零依赖）。
//! 单 crate 起步、预留拆分形状（将来机械裂出 `contract-agent` 等）。
//!
//! 命名空间：`shared`（双向共享，中立）+ 按服务分目录（`agent`、`asset`
//! …，服务落地时逐个加入，消息归调用方）。

pub mod shared;
