//! contracts — 全部服务 Message 的单一 crate（design D2）。
//!
//! 归属规则：**消息归调用方**（调用方拥有自己发的消息与期待的回复类型，
//! 放自己服务的命名空间），被调方 import 调用方契约；**双向共享的消息类型
//! 归 `contracts::shared` 中立命名空间**（裂分时 = 独立中立 crate，零依赖）。
//! 这是将来机械裂出 `contract-agent` 等独立 crate 而无循环依赖的前提。
