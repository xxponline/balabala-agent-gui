//! legacy `.workspaces.json` 只读迁移源 (change: single-workspace-
//! project-model — app 根的 workspace 列表格式退役)。
//!
//! 职责只剩两件 (workspace-management: Legacy .workspaces.json migration):
//! 1. 播种全局历史 (`seed_legacy`, 由 `legacy_seeded` 标志保证一次性);
//! 2. 按需补拷 `file_tree` 规则到工作区自己的配置 (`legacy_rules_for`,
//!    工作区配置缺区段时调用)。
//!
//! **本模块永不写 legacy 文件**; 原 `save`/`save_at` 写路径已随
//! workspace 列表持久化一并移除 (task 6.2)。

use std::path::{Path, PathBuf};

use crate::workspace::history::History;

/// legacy 持久化文件 (app 根; 当前进程 cwd 即 app 根)。
pub fn legacy_path() -> PathBuf {
    PathBuf::from(".workspaces.json")
}

/// legacy 文件的完整形状 (列表 + 按 workspace 路径键控的规则区段)。
#[derive(serde::Deserialize, Default)]
pub(crate) struct LegacyPersisted {
    #[serde(default)]
    pub workspaces: Vec<PathBuf>,
    #[serde(default)]
    pub file_tree: std::collections::HashMap<String, Vec<String>>,
}

/// 容错读: 缺失/损坏 → None (legacy 不再承重, 坏文件不再阻断启动 —
/// 旧的"损坏即退出"契约已移到工作区 marker)。
pub(crate) fn read_legacy(path: &Path) -> Option<LegacyPersisted> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|c| serde_json::from_str(&c).ok())
}

/// 一次性播种历史: 未播种 → 置 `legacy_seeded` 并按文件顺序把 legacy
/// 列表 push 进 `recent` (列表尾 = 最新添加 → push 顺序自然使后者居首)。
///
/// 返回 `true` 表示历史有变化 (调用方负责落盘 — 即使 legacy 不存在也要
/// 记录 seeded, 避免每次启动重读)。legacy 解析失败: 跳过播种但仍记
/// seeded (规则补拷另行按需容错读)。
pub fn seed_legacy(history: &mut History, legacy_file: &Path) -> bool {
    if history.legacy_seeded {
        return false; // 已播种 — 不再读 legacy (spec: once)
    }
    history.legacy_seeded = true;
    if let Some(legacy) = read_legacy(legacy_file) {
        for path in legacy.workspaces {
            history.push(path);
        }
    }
    true
}

/// legacy 里该 workspace 的 `file_tree` 规则 (迁移补拷的数据源)。
/// 缺失/损坏/无该键/空数组 → `None` (无可拷, 不写工作区配置)。
pub fn legacy_rules_for(legacy_file: &Path, workspace_path: &Path) -> Option<Vec<String>> {
    let legacy = read_legacy(legacy_file)?;
    let rules = legacy
        .file_tree
        .get(workspace_path.to_string_lossy().as_ref())?;
    if rules.is_empty() {
        None
    } else {
        Some(rules.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "legacy-{name}-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn seed_merges_legacy_list_once_and_never_writes_legacy() {
        let legacy = temp_file("seed");
        let seed_bytes = br#"{ "workspaces": ["/a", "/b"], "file_tree": { "/a": ["*.log"] } }"#;
        std::fs::write(&legacy, seed_bytes).unwrap();

        let mut h = History::default();
        assert!(seed_legacy(&mut h, &legacy), "首次播种 → 需要落盘");
        assert!(h.legacy_seeded);
        assert_eq!(
            h.recent,
            vec![PathBuf::from("/b"), PathBuf::from("/a")],
            "按文件顺序 push → 后添加者居首"
        );

        // legacy 字节不变 (spec: SHALL NOT rewrite the legacy file)
        assert_eq!(std::fs::read(&legacy).unwrap(), seed_bytes);

        // 第二次: 无变化 (已 seeded), 即使 legacy 内容后来变了也不再读
        std::fs::write(&legacy, r#"{ "workspaces": ["/c"] }"#).unwrap();
        let mut h2 = h.clone();
        assert!(!seed_legacy(&mut h2, &legacy), "seeded 后不再读 legacy");
        assert_eq!(h2.recent, h.recent);

        let _ = std::fs::remove_file(&legacy);
    }

    #[test]
    fn seed_with_corrupt_legacy_still_marks_seeded() {
        let legacy = temp_file("corrupt");
        std::fs::write(&legacy, "{ nope").unwrap();
        let mut h = History::default();
        assert!(seed_legacy(&mut h, &legacy));
        assert!(h.legacy_seeded, "损坏也记 seeded (不重试循环)");
        assert!(h.recent.is_empty());
        let _ = std::fs::remove_file(&legacy);
    }

    #[test]
    fn seed_without_legacy_file_only_sets_flag() {
        let missing = temp_file("missing");
        let mut h = History::default();
        assert!(seed_legacy(&mut h, &missing));
        assert!(h.legacy_seeded);
        assert!(h.recent.is_empty());
    }

    #[test]
    fn legacy_rules_lookup_is_keyed_and_tolerant() {
        let legacy = temp_file("rules");
        std::fs::write(
            &legacy,
            r#"{ "workspaces": ["/ws"], "file_tree": { "/ws": ["*.log", "!keep.log"] } }"#,
        )
        .unwrap();
        assert_eq!(
            legacy_rules_for(&legacy, Path::new("/ws")),
            Some(vec!["*.log".to_string(), "!keep.log".to_string()])
        );
        assert_eq!(legacy_rules_for(&legacy, Path::new("/other")), None);
        assert_eq!(legacy_rules_for(Path::new("/nonexistent"), Path::new("/ws")), None);
        let _ = std::fs::remove_file(&legacy);
    }

    #[test]
    fn empty_legacy_rules_entry_means_nothing_to_copy() {
        let legacy = temp_file("empty-rules");
        std::fs::write(&legacy, r#"{ "workspaces": [], "file_tree": { "/ws": [] } }"#).unwrap();
        assert_eq!(legacy_rules_for(&legacy, Path::new("/ws")), None);
        let _ = std::fs::remove_file(&legacy);
    }
}
