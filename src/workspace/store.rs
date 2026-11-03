//! `.workspaces.json` 持久化读写。
//!
//! 文件格式:
//! ```json
//! { "workspaces": ["/Users/me/code/foo", "/Users/me/code/bar"] }
//! ```

use std::path::PathBuf;

#[derive(serde::Serialize, serde::Deserialize, Default)]
pub struct Persisted {
    pub workspaces: Vec<PathBuf>,
}

/// 持久化文件的位置: 项目根目录下的 `.workspaces.json`。
///
/// 当前进程的 cwd 就是项目根 (cargo run 时)。
fn filepath() -> PathBuf {
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
pub fn save(paths: &[PathBuf]) -> Result<(), String> {
    let path = filepath();
    let persisted = Persisted {
        workspaces: paths.to_vec(),
    };
    let json = serde_json::to_string_pretty(&persisted)
        .map_err(|e| format!("serialize: {e}"))?;
    std::fs::write(&path, json)
        .map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(())
}
