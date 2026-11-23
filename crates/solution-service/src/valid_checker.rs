//! `ValidChecker`——幂等工具集（design D6）：create 同路径去重判定、
//! configure 同值 no-op 判定等纯函数。crate 根导出，View 层（发送前置检）
//! 与 Server 层（处理侧校验）共用同一实现，保证两端幂等语义一致
//! （spec: Create and configure operations are idempotent）。

use crate::contract::SolutionItem;
use std::path::Path;

/// create 幂等判定：该 path 的 solution 已存在 → 重复请求（不应再创建，
/// spec: Re-adding an existing solution changes nothing）。
pub fn create_is_noop(items: &[SolutionItem], path: &Path) -> bool {
    items.iter().any(|i| i.path() == path)
}

/// configure 幂等判定：目标存在且所有非 `None` 请求字段均已是请求值 →
/// no-op（spec: Re-applying the same configuration is a no-op）。
///
/// 目标不存在返回 `false`——由调用方的 `NoSuch` 分支承接（本函数只判幂等，
/// 不判存在性错误）。
pub fn configure_is_noop(
    items: &[SolutionItem],
    target: &Path,
    register_to_asset_manager: Option<bool>,
    git_enabled: Option<bool>,
) -> bool {
    let Some(item) = items.iter().find(|i| i.path() == target) else {
        return false;
    };
    let same_register =
        register_to_asset_manager.is_none_or(|v| item.register_to_asset_manager() == v);
    let same_git = git_enabled.is_none_or(|v| item.git_enabled() == v);
    same_register && same_git
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn item(path: &str, register: bool, git: bool) -> SolutionItem {
        SolutionItem::Simple {
            path: PathBuf::from(path),
            register_to_asset_manager: register,
            git_enabled: git,
        }
    }

    /// 幂等矩阵（task 3.1 验证面）：同路径/同值/先验后验。
    #[test]
    fn create_dedup_matrix() {
        let items = vec![item("/a/foo", true, true)];
        // 同路径 → 重复 (no-op)
        assert!(create_is_noop(&items, Path::new("/a/foo")));
        // 新路径 → 可创建
        assert!(!create_is_noop(&items, Path::new("/a/bar")));
        // 空集 → 可创建
        assert!(!create_is_noop(&[], Path::new("/a/foo")));
    }

    #[test]
    fn configure_noop_matrix() {
        let items = vec![item("/a/foo", true, false)];
        // 同值 (全字段) → no-op
        assert!(configure_is_noop(
            &items,
            Path::new("/a/foo"),
            Some(true),
            Some(false)
        ));
        // 同值 (部分字段 / None = 不改) → no-op
        assert!(configure_is_noop(&items, Path::new("/a/foo"), Some(true), None));
        assert!(configure_is_noop(&items, Path::new("/a/foo"), None, None));
        // 值不同 → 变更
        assert!(!configure_is_noop(
            &items,
            Path::new("/a/foo"),
            Some(false),
            None
        ));
        assert!(!configure_is_noop(
            &items,
            Path::new("/a/foo"),
            None,
            Some(true)
        ));
        // 目标不存在 → 非 no-op (交给 NoSuch 分支)
        assert!(!configure_is_noop(&items, Path::new("/a/gone"), None, None));
    }
}
