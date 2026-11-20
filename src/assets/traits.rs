//! 枚举契约 — 资产侧与 project 域的唯一边界 (design D2/D3: Level 2)。
//!
//! 本文件是 `project_model -> assets` 依赖方向的锚: 资产侧只认识这两个
//! trait 与 [`state`] DTO, 不认识 Project/规则引擎 — 实现见
//! `crate::project_model::enumerate`。

use std::future::Future;
use std::path::Path;
use std::pin::Pin;

use gpui_kit::BackgroundExecutor;

use super::state::AssetNode;

/// 异步枚举结果的盒装 future (BoxFuture 别名; 零新依赖)。
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// 可枚举资产的实体 — 资产侧唯一的资产源视图 (design D2)。
///
/// 链上不携带过滤输入 — 过滤规则由 Project 在枚举需要 Module 时
/// 构建并注入 (design D2/D8), 所有输入从 `root` 自身推导。
pub trait EnumeratorOwner {
    /// 身份根路径: 注册记账 "身份 = root" 与 reveal 语义的依据。
    ///
    /// 输入: 无 (`&self`); 输出: 资产树根目录的绝对路径 (Project 根 /
    /// 模块根)。
    fn root(&self) -> &Path;

    /// 为该实体构造枚举执行体 (同步构造, 由调用方丢入后台 spawn)。
    ///
    /// 输入: 无 (`&self`); 输出: 异步产出该实体资产子树的枚举器。
    fn enumerator(&self) -> Box<dyn Enumerator>;
}

/// 资产枚举器: 异步产出该实体的完整资产子树。
///
/// `bg` 供阻塞 fs 工作丢后台执行器; future 为 `'static`, 由 manager 在
/// 自己的 spawn 里驱动 (design D2: 一次 spawn 内组装, 原子 land)。
pub trait Enumerator: Send {
    /// 执行枚举 (消耗执行体)。
    ///
    /// 输入: 后台执行器; 输出: 资产子树 (`Err` 原因由 manager 落降级态)。
    fn enumerate(
        self: Box<Self>,
        bg: BackgroundExecutor,
    ) -> BoxFuture<'static, Result<AssetNode, String>>;
}
