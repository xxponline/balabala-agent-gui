//! workspace 文件契约 (design D3): marker 最后写, marker 是成立唯一真值。
//!
//! - `.balabala.workspace.json` = 身份 (`version` 必有; `file_tree`
//!   规则区段已随过滤源迁到项目根 `.gitignore` 退役 — 残留字段被
//!   serde 忽略, 无数据迁移)。
//! - `balabala.solutions.json` = solution items 数组 (缺失 = 合法空工作区,
//!   spec: Solution configuration file)。
//! - 创建顺序: **先 solutions 后 marker** — 任一步失败都不留"半个工作区"
//!   (marker 不在 = 目录还不算工作区, 重试直接覆盖残留)。

use std::path::{Path, PathBuf};

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

/// marker 文件内容: 仅身份 (version) — 过滤规则源在项目根
/// `.gitignore` (file-filter-prune D9), 不再进工作区配置。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WorkspaceMarker {
    pub version: u32,
}

impl WorkspaceMarker {
    pub fn new() -> Self {
        Self {
            version: MARKER_VERSION,
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

// solution items 的解析/追加已随 solution 服务迁移 (change
// solution-service 5.2: 创建走服务 Create ask, 解析归服务 loader)。

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
        assert_eq!(
            fs::read_to_string(solutions_path(&dir)).unwrap().trim(),
            "[]",
            "创建即写空数组"
        );
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
}
