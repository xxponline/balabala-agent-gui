//! `.workspaces.json` 持久化读写。
//!
//! 文件格式 (workspace-management spec: 可携带其他区段):
//! ```json
//! {
//!   "workspaces": ["/Users/me/code/foo"],
//!   "file_tree": { "/Users/me/code/foo": ["*.log", "!keep.log"] }
//! }
//! ```

use std::path::{Path, PathBuf};

#[derive(serde::Serialize, serde::Deserialize, Default)]
pub struct Persisted {
    pub workspaces: Vec<PathBuf>,
    /// 文件树过滤规则区段 (file-tree spec: Workspace persistence 修订):
    /// workspace 绝对路径 -> gitignore 式模式行数组。空省略 (旧格式字节不变)。
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub file_tree: std::collections::HashMap<String, Vec<String>>,
}

/// 持久化文件的位置: 项目根目录下的 `.workspaces.json`。
///
/// 当前进程的 cwd 就是项目根 (cargo run 时)。
pub fn filepath() -> PathBuf {
    PathBuf::from(".workspaces.json")
}

/// 从 `.workspaces.json` 读取 workspace 路径列表。
///
/// - 文件不存在 → Ok(空列表)
/// - 文件存在但读 / 解析失败 → Err(错误信息)
pub fn load() -> Result<Vec<PathBuf>, String> {
    let path = filepath();
    if !path.exists() {
        return Ok(Vec::new());
    }
    let contents = std::fs::read_to_string(&path)
        .map_err(|e| format!("read {}: {e}", path.display()))?;
    let persisted: Persisted = serde_json::from_str(&contents)
        .map_err(|e| format!("parse {}: {e}", path.display()))?;
    Ok(persisted.workspaces)
}

/// 把 workspace 路径列表写入 `.workspaces.json`。
///
/// **保全区段** (spec: Filter section survives workspace mutation):
/// 先读现有文件解析出完整 `Persisted`（含 `file_tree` 等非本变更拥有的
/// 区段），只替换 `workspaces` 后写回；文件缺失/损坏时退回纯列表
/// （损坏文件会在下次启动的 load 契约里被拦截）。
pub fn save(paths: &[PathBuf]) -> Result<(), String> {
    save_at(&filepath(), paths)
}

/// 同 `save`，显式路径（测试缝：不落用户真实配置文件）。
pub fn save_at(path: &Path, paths: &[PathBuf]) -> Result<(), String> {
    let mut persisted = std::fs::read_to_string(path)
        .ok()
        .and_then(|contents| serde_json::from_str::<Persisted>(&contents).ok())
        .unwrap_or_default();
    persisted.workspaces = paths.to_vec();
    let json = serde_json::to_string_pretty(&persisted)
        .map_err(|e| format!("serialize: {e}"))?;
    std::fs::write(path, json)
        .map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(())
}

/// 读取文件树过滤规则区段 (`file_tree`)：workspace 路径 -> 模式行数组。
///
/// 文件缺失 / 解析失败 / 区段缺失 → 空 map（规则层按"空 = 全显"处理；
/// 启动时的解析失败已由 `load` 的错误契约拦截，这里只服务运行期重读）。
pub fn load_file_tree_rules(config: &Path) -> std::collections::HashMap<String, Vec<String>> {
    std::fs::read_to_string(config)
        .ok()
        .and_then(|contents| serde_json::from_str::<Persisted>(&contents).ok())
        .map(|persisted| persisted.file_tree)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_config(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("store-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(format!("{name}.json"))
    }

    #[test]
    fn store_roundtrip_preserves_file_tree() {
        let path = temp_config("roundtrip");
        // 带区段的现有配置
        let seed = serde_json::json!({
            "workspaces": ["/a/ws"],
            "file_tree": { "/a/ws": ["*.log", "!keep.log"] }
        });
        std::fs::write(&path, seed.to_string()).unwrap();

        // 增删 workspace 触发写回
        save_at(&path, &["/a/ws".into(), "/b/ws".into()]).unwrap();

        let raw = std::fs::read_to_string(&path).unwrap();
        let back: Persisted = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            back.workspaces,
            vec![PathBuf::from("/a/ws"), PathBuf::from("/b/ws")],
            "workspaces 应更新"
        );
        assert_eq!(
            back.file_tree.get("/a/ws"),
            Some(&vec!["*.log".to_string(), "!keep.log".to_string()]),
            "file_tree 区段必须在写回中保全"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn store_old_format_still_loads() {
        let path = temp_config("old-format");
        std::fs::write(&path, r#"{ "workspaces": ["/x"] }"#).unwrap();
        let rules = load_file_tree_rules(&path);
        assert!(rules.is_empty(), "旧格式无区段 → 空规则");

        save_at(&path, &[]).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(
            !raw.contains("file_tree"),
            "无规则时区段应省略 (旧格式字节不变)"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn store_rules_lookup_by_workspace() {
        let path = temp_config("lookup");
        let seed = serde_json::json!({
            "workspaces": [],
            "file_tree": { "/rust/ws": ["*.rs"] }
        });
        std::fs::write(&path, seed.to_string()).unwrap();
        let rules = load_file_tree_rules(&path);
        assert_eq!(rules.get("/rust/ws"), Some(&vec!["*.rs".to_string()]));
        assert!(rules.get("/other").is_none());
        let _ = std::fs::remove_file(&path);
    }
}
