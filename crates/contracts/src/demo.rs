//! demo 服务的契约消息（示例插件 / 集成测试载体）。
//!
//! **归属规则演示（design D2）**：`Ping`/`Pong` 是单向问答——**归调用方**
//! （宿主/集成测试发问、期待 `Pong`），被调方（示例插件）import 本命名空间
//! 来构造回复。若出现双向共享类型，才入 `contracts::shared`。

/// 请求：宿主 → demo 服务（调用方拥有）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ping(pub u64);

/// 回复：期待的应答类型（同样归调用方，D2 口径）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pong(pub u64);
