//! 契约身份（change `singleton-per-contract`, design D1）：主骨架的"类"判据。
//!
//! 主骨架下每个契约在实例表中至多一个服务实例（spec L:
//! Single-instance loading per contract）；类 = 契约 marker 类型的
//! [`TypeId`]，包装为 [`ContractId`]。身份一律由**宿主**铸造
//! （[`ContractId::of`]），不经 dylib 边界传递 `TypeId` 数值——与 payload
//! downcast 依赖的"零依赖契约 crate 共享同一 rlib ⇒ TypeId 稳定"是同一支柱。

use std::any::TypeId;

/// 契约身份：契约 marker 类型 `TypeId` 的新类型包装（Copy/Eq/Hash 值语义）。
///
/// 功能：区分"哪一类服务"——同 marker 即同类，骨架内恰一实例。
/// 输入：经 [`ContractId::of`] 以任意 `'static` marker 类型铸造。
/// 输出：可作实例表索引键的稳定身份；`Debug` 输出为底层 `TypeId`。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ContractId(TypeId);

impl ContractId {
    /// 以契约 marker 类型铸造身份（宿主侧唯一铸造入口）。
    ///
    /// 功能：把 marker 类型 `T` 的 `TypeId` 包装为契约身份。
    /// 输入：`T` — 契约 marker 类型（零大小标记结构体，只承载身份、无行为）。
    /// 输出：同一 `T` 多次铸造恒等、不同 `T` 互异的 [`ContractId`]。
    pub fn of<T: ?Sized + 'static>() -> Self {
        ContractId(TypeId::of::<T>())
    }

    /// 底层 `TypeId`（诊断/日志用）。
    ///
    /// 功能：取出原始 `TypeId`。
    /// 输入：无（`self` 按值复制）。
    /// 输出：与铸造时同一 marker 的 `TypeId`；**不得序列化跨进程传递**。
    pub fn type_id(self) -> TypeId {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// 测试 marker：契约 A。
    struct MarkerA;
    /// 测试 marker：契约 B。
    struct MarkerB;

    /// 同一 marker 多次铸造恒等，不同 marker 互异（类判据的基本约定）。
    ///
    /// 功能：断言 `ContractId::of` 的恒等/互异性。
    /// 输入：无。
    /// 输出：断言通过。
    #[test]
    fn same_marker_is_same_contract_distinct_markers_differ() {
        assert_eq!(ContractId::of::<MarkerA>(), ContractId::of::<MarkerA>());
        assert_ne!(ContractId::of::<MarkerA>(), ContractId::of::<MarkerB>());
    }

    /// `ContractId` 按 marker 类型落入哈希桶（实例表索引键用途）。
    ///
    /// 功能：断言身份值可作 `HashMap` 键且两个 marker 各占一桶。
    /// 输入：无。
    /// 输出：断言通过。
    #[test]
    fn contract_id_hashes_by_marker() {
        let mut map = HashMap::new();
        map.insert(ContractId::of::<MarkerA>(), 1u32);
        map.insert(ContractId::of::<MarkerB>(), 2u32);
        map.insert(ContractId::of::<MarkerA>(), 3u32);
        assert_eq!(map.len(), 2, "同 marker 单桶");
        assert_eq!(map[&ContractId::of::<MarkerA>()], 3, "同 marker 覆盖写");
    }
}
