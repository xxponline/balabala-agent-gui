//! 枚举契约 — 资产侧与 project 域的唯一边界 (design D2; 组2 收紧 Level 2)。
//!
//! 本文件是 `project_model -> assets` 依赖方向的锚: 资产侧只认识这两个
//! trait 与 [`state`] DTO, 不认识 Project/规则引擎 — 实现见
//! `crate::project_model::enumerate`。

use std::future::Future;
use std::pin::Pin;

use gpui_kit::BackgroundExecutor;

use super::state::AssetNode;

/// 异步枚举结果的盒装 future (BoxFuture 别名; 零新依赖)。
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// 可枚举资产的实体 (Project / Module)。
///
/// 链上不携带过滤输入 — 过滤规则由 Project 在枚举需要 Module 时
/// 构建并注入 (design D2/D8), 所有输入从 `root` 自身推导。
pub trait AssetEnumeratable {
    fn enumerator(&self) -> Box<dyn AssetEnumerator>;
}

/// 资产枚举器: 异步产出该实体的完整资产子树。
///
/// `bg` 供阻塞 fs 工作丢后台执行器; future 为 `'static`, 由 manager 在
/// 自己的 spawn 里驱动 (design D2: 一次 spawn 内组装, 原子 land)。
pub trait AssetEnumerator: Send {
    fn enumerate(
        self: Box<Self>,
        bg: BackgroundExecutor,
    ) -> BoxFuture<'static, Result<AssetNode, String>>;
}

