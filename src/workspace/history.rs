//! 全局 workspace 历史 (design D4): most-recent-first, 存应用数据目录。
//!
//! 文件: `<ProjectDirs>/workspace_history.json` — **不属于任何工作区**
//! (单工作区后"列表"只剩 MRU; 放全局才不违反"配置都在工作区里")。
//! 历史是便利数据不是身份数据: 损坏 → 按无历史处理, 不退出 (D4)。
//!
//! 一次性迁移: legacy `.workspaces.json` (app 根) 播种历史, `legacy_seeded`
//! 标志落盘后不再读它做播种 (legacy 文件永不被改写 —
//! workspace-management: Legacy migration)。

use std::path::{Path, PathBuf};

/// 历史文件格式版本。
pub const HISTORY_VERSION: u32 = 1;
/// 封顶条数 (design D4 建议 10)。
pub const MAX_RECENT: usize = 10;
/// 历史文件名。
#[cfg_attr(test, allow(dead_code))] // 仅 cfg(not(test)) 分支引用 (default_path)
pub const HISTORY_FILE: &str = "workspace_history.json";

/// 全局历史文件路径。
///
/// - 正式: 平台应用数据目录 (macOS: `~/Library/Application Support/
///   balabala-agent-gui/`), `directories` crate 解析。
/// - 测试 (`cfg(test)`): 进程独享临时路径 — 各测试经 `load_at/save_at`
///   显式路径隔离, 默认路径只服务 `TerminalApp::new` 的"无历史 → 空态"
///   前提 (同进程共享, 不用于断言播种/写入)。
pub fn default_path() -> PathBuf {
    #[cfg(test)]
    {
        std::env::temp_dir().join(format!("balabala-history-{}.json", std::process::id()))
    }
    #[cfg(not(test))]
    {
        directories::ProjectDirs::from("com.balabala", "", "balabala-agent-gui")
            .map(|d| d.data_dir().join(HISTORY_FILE))
            .unwrap_or_else(|| PathBuf::from(HISTORY_FILE))
    }
}

/// 历史内容 (`recent` 最近打开在前)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct History {
    pub version: u32,
    /// legacy 播种已完成标志 (完成即不再读 legacy 做播种)。
    #[serde(default)]
    pub legacy_seeded: bool,
    #[serde(default)]
    pub recent: Vec<PathBuf>,
}

impl Default for History {
    fn default() -> Self {
        Self {
            version: HISTORY_VERSION,
            legacy_seeded: false,
            recent: Vec::new(),
        }
    }
}

impl History {
    /// 读历史: 文件缺失/解析失败/版本不认识 → 默认空历史 (便利数据,
    /// spec-compatible with "no history → picker empty state")。
    pub fn load_at(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|c| serde_json::from_str::<History>(&c).ok())
            .filter(|h| h.version == HISTORY_VERSION)
            .unwrap_or_default()
    }

    pub fn save_at(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
            }
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| format!("serialize: {e}"))?;
        std::fs::write(path, json).map_err(|e| format!("write {}: {e}", path.display()))
    }

    /// 记一次打开: 置顶 + 去重 + 封顶。
    pub fn push(&mut self, dir: PathBuf) {
        self.recent.retain(|p| *p != dir);
        self.recent.insert(0, dir);
        self.recent.truncate(MAX_RECENT);
    }

    /// most-recent-first 的第一条 (启动直开目标)。
    pub fn most_recent(&self) -> Option<&Path> {
        self.recent.first().map(|p| p.as_path())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "ws-history-{name}-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn missing_and_corrupt_history_are_empty_not_fatal() {
        let path = temp_file("corrupt");
        assert_eq!(History::load_at(&path), History::default(), "缺失 = 空");
        std::fs::write(&path, "{ nope").unwrap();
        assert_eq!(
            History::load_at(&path),
            History::default(),
            "损坏 = 空 (便利数据不退出, D4)"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn push_dedupes_front_and_caps() {
        let mut h = History::default();
        h.push("/a".into());
        h.push("/b".into());
        h.push("/a".into()); // 再开 /a → 置顶不重复
        assert_eq!(h.recent, vec![PathBuf::from("/a"), PathBuf::from("/b")]);
        for i in 0..MAX_RECENT + 5 {
            h.push(format!("/x{i}").into());
        }
        assert_eq!(h.recent.len(), MAX_RECENT, "封顶 {MAX_RECENT}");
        assert_eq!(h.most_recent(), Some(Path::new("/x14")));
    }

    #[test]
    fn roundtrip_preserves_flag_and_order() {
        let path = temp_file("roundtrip");
        let mut h = History::default();
        h.legacy_seeded = true;
        h.push("/first".into());
        h.push("/second".into());
        h.save_at(&path).unwrap();
        let back = History::load_at(&path);
        assert_eq!(back, h);
        assert_eq!(back.most_recent(), Some(Path::new("/second")));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn default_path_is_isolated_under_tests() {
        // cfg(test) 路径必须落在临时目录 — 不能碰真实应用数据目录;
        // 文件名是进程独享的 balabala-history-<pid>.json (同进程测试共享,
        // 只读不写 — 写入一律走 load_at/save_at 显式路径)
        let p = default_path();
        assert!(p.starts_with(std::env::temp_dir()));
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            name.starts_with("balabala-history-") && name.ends_with(".json"),
            "意外的历史文件名: {name}"
        );
    }
}
