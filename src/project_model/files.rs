//! workspace 文件契约 (design D3): marker 最后写, marker 是成立唯一真值。
//!
//! - `.balabala.workspace.json` = 身份 + 配置 (`version` 必有, `file_tree`
//!   规则区段按需省略 — 单工作区, 规则不再按路径键控)。
//! - `balabala.solutions.json` = solution items 数组 (缺失 = 合法空工作区,
//!   spec: Solution configuration file)。
//! - 创建顺序: **先 solutions 后 marker** — 任一步失败都不留"半个工作区"
//!   (marker 不在 = 目录还不算工作区, 重试直接覆盖残留)。

use std::path::{Path, PathBuf};

use super::solution::SolutionItem;

/// 工作区 marker 文件名 (身份 + 配置)。
pub const MARKER_FILE: &str = ".balabala.workspace.json";
/// solution 配置文件名。
pub const SOLUTIONS_FILE: &str = "balabala.solutions.json";
/// 当前 marker 格式版本 (只认这个; 更高版本 = 不可解析, 走错误契约)。
pub const MARKER_VERSION: u32 = 1;

pub fn marker_path(workspace_dir: &Path) -> PathBuf {
    workspace_dir.join(MARKER_FILE)
}

pub fn solutions_path(workspace_dir: &Path) -> PathBuf {
    workspace_dir.join(SOLUTIONS_FILE)
}

/// marker 文件内容: 身份 (version) + 工作区配置 (file_tree 规则)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WorkspaceMarker {
    pub version: u32,
    /// gitignore 式过滤规则 (file-tree spec: 来自工作区自己的配置文件)。
    /// 空省略 — 保持无规则时的字节稳定。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub file_tree: Vec<String>,
}

impl WorkspaceMarker {
    pub fn new() -> Self {
        Self {
            version: MARKER_VERSION,
            file_tree: Vec::new(),
        }
    }
}

impl Default for WorkspaceMarker {
    fn default() -> Self {
        Self::new()
    }
}

/// marker 三态 (spec: Workspace opening via directory picker)。
#[derive(Debug, Clone, PartialEq)]
pub enum MarkerStatus {
    /// 文件不存在 → 询问是否创建工作区。
    Missing,
    /// 存在但解析失败 / 版本不认识 → 错误框 + 退出。
    Corrupt(String),
    /// 合法 → 打开 (随后加载 solutions, 走 ModelError 契约)。
    Valid(WorkspaceMarker),
}

/// 读取并判定 marker。
pub fn marker_status(workspace_dir: &Path) -> MarkerStatus {
    let path = marker_path(workspace_dir);
    if !path.exists() {
        return MarkerStatus::Missing;
    }
    let contents = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => return MarkerStatus::Corrupt(format!("read {}: {e}", path.display())),
    };
    match serde_json::from_str::<WorkspaceMarker>(&contents) {
        Ok(marker) if marker.version == MARKER_VERSION => MarkerStatus::Valid(marker),
        Ok(marker) => MarkerStatus::Corrupt(format!(
            "{}: unsupported version {} (expected {MARKER_VERSION})",
            path.display(),
            marker.version
        )),
        Err(e) => MarkerStatus::Corrupt(format!("parse {}: {e}", path.display())),
    }
}

/// 新建工作区: 写空 solutions, **成功后**写 marker (D3: marker 最后)。
///
/// 任一步失败返回 Err 且保证 marker 不存在 — 无半工作区 (spec:
/// Workspace creation writes identity and solutions files)。
pub fn create_workspace_files(workspace_dir: &Path) -> Result<(), String> {
    if !workspace_dir.is_dir() {
        return Err(format!("{} is not a directory", workspace_dir.display()));
    }
    // 1. solutions 先行 (失败 → 无 marker, 残留无害, 重试覆盖)
    std::fs::write(solutions_path(workspace_dir), "[]\n")
        .map_err(|e| format!("write {}: {e}", solutions_path(workspace_dir).display()))?;
    // 2. marker 收尾 = 工作区成立
    let marker_json = serde_json::to_string_pretty(&WorkspaceMarker::new())
        .map_err(|e| format!("serialize marker: {e}"))?;
    std::fs::write(marker_path(workspace_dir), marker_json)
        .map_err(|e| format!("write {}: {e}", marker_path(workspace_dir).display()))?;
    Ok(())
}

/// 追加一个 `simple` solution item (保留既有 items 与顺序)。
///
/// solutions 文件缺失视作空数组 (spec: 缺失 = 合法空工作区)。
/// 调用方负责写失败时的红条契约 (workspace-management: Persistence
/// write failure surfacing)。
pub fn append_simple_solution(
    workspace_dir: &Path,
    path: PathBuf,
) -> Result<Vec<SolutionItem>, String> {
    let mut items = load_solutions(workspace_dir)
        .map_err(|e| format!("{}: {e}", solutions_path(workspace_dir).display()))?;
    items.push(SolutionItem::Simple { path });
    let json = serde_json::to_string_pretty(&items).map_err(|e| format!("serialize: {e}"))?;
    std::fs::write(solutions_path(workspace_dir), json)
        .map_err(|e| format!("write {}: {e}", solutions_path(workspace_dir).display()))?;
    Ok(items)
}

/// 读 solution items: 文件缺失 → 空 (合法); 读/解析失败 → Err。
pub(super) fn load_solutions(workspace_dir: &Path) -> Result<Vec<SolutionItem>, String> {
    let path = solutions_path(workspace_dir);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let contents =
        std::fs::read_to_string(&path).map_err(|e| format!("read {path:?}: {e}"))?;
    serde_json::from_str(&contents).map_err(|e| format!("parse {path:?}: {e}"))
}

/// 读工作区 `file_tree` 规则 (file-tree spec: 刷新期容错读 — 缺失/解析
/// 失败 = 空规则 = 全显; 启动期的坏文件契约已由打开路径拦截)。
pub fn load_file_tree_rules(workspace_dir: &Path) -> Vec<String> {
    match marker_status(workspace_dir) {
        MarkerStatus::Valid(marker) => marker.file_tree,
        _ => Vec::new(),
    }
}

/// 把规则写进 marker (保留 version; legacy 迁移的补拷与未来配置写入用)。
/// 文件缺失/损坏 → Err (由调用方决定是否落红条)。
pub fn set_file_tree_rules(workspace_dir: &Path, rules: Vec<String>) -> Result<(), String> {
    let mut marker = match marker_status(workspace_dir) {
        MarkerStatus::Valid(m) => m,
        MarkerStatus::Missing => WorkspaceMarker::new(),
        MarkerStatus::Corrupt(e) => return Err(e),
    };
    marker.file_tree = rules;
    let json = serde_json::to_string_pretty(&marker).map_err(|e| format!("serialize: {e}"))?;
    std::fs::write(marker_path(workspace_dir), json)
        .map_err(|e| format!("write {}: {e}", marker_path(workspace_dir).display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pm-files-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn create_writes_marker_last_and_both_present() {
        let dir = temp_dir("create");
        create_workspace_files(&dir).unwrap();
        assert!(marker_path(&dir).exists(), "marker 应存在");
        assert!(
            solutions_path(&dir).exists(),
            "solutions 应存在 (创建即写空数组)"
        );
        assert_eq!(load_solutions(&dir).unwrap(), Vec::new());
        match marker_status(&dir) {
            MarkerStatus::Valid(m) => assert_eq!(m.version, MARKER_VERSION),
            other => panic!("marker 应合法, got {other:?}"),
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn create_failure_leaves_no_marker() {
        let dir = temp_dir("create-fail");
        // 让 solutions 写入必败: 预置同名目录
        fs::create_dir(solutions_path(&dir)).unwrap();
        let err = create_workspace_files(&dir).unwrap_err();
        assert!(err.contains("write"), "错误应点名写入失败: {err}");
        assert!(
            !marker_path(&dir).exists(),
            "写失败后不得有 marker (无半工作区)"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn marker_missing_and_corrupt_states() {
        let dir = temp_dir("marker-state");
        assert_eq!(marker_status(&dir), MarkerStatus::Missing);

        fs::write(marker_path(&dir), "{ not json").unwrap();
        assert!(matches!(marker_status(&dir), MarkerStatus::Corrupt(_)));

        fs::write(marker_path(&dir), r#"{ "version": 99 }"#).unwrap();
        assert!(
            matches!(marker_status(&dir), MarkerStatus::Corrupt(_)),
            "未知版本 = 不可解析 (错误契约)"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn append_preserves_existing_items() {
        let dir = temp_dir("append");
        create_workspace_files(&dir).unwrap();
        append_simple_solution(&dir, "/a/one".into()).unwrap();
        append_simple_solution(&dir, "/a/two".into()).unwrap();
        let items = load_solutions(&dir).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0], SolutionItem::Simple { path: "/a/one".into() });
        assert_eq!(items[1], SolutionItem::Simple { path: "/a/two".into() });
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn append_works_when_solutions_file_missing() {
        let dir = temp_dir("append-missing");
        create_workspace_files(&dir).unwrap();
        fs::remove_file(solutions_path(&dir)).unwrap();
        append_simple_solution(&dir, "/a/x".into()).unwrap();
        assert_eq!(load_solutions(&dir).unwrap().len(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_tree_rules_roundtrip_via_marker() {
        let dir = temp_dir("rules");
        create_workspace_files(&dir).unwrap();
        assert!(load_file_tree_rules(&dir).is_empty(), "无区段 = 空规则");
        set_file_tree_rules(&dir, vec!["*.log".into(), "!keep.log".into()]).unwrap();
        assert_eq!(
            load_file_tree_rules(&dir),
            vec!["*.log".to_string(), "!keep.log".to_string()]
        );
        // solutions 区段不受影响 (两个文件各管各的)
        assert!(solutions_path(&dir).exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
