//! Scanner：启动期扫服务目录建条目（design D8——无 watch，显式 reload）。

use crate::catalog::ServiceDescriptor;
use std::io;
use std::path::{Path, PathBuf};

/// 可识别的服务产物扩展名（平台随编译目标）。
pub const PLUGIN_EXTENSIONS: &[&str] = &["dylib", "so", "dll"];

/// 扫描错误（目录不可读等；**目录不存在 = 空**，plugins/ 首次构建可能未产出）。
#[derive(Debug)]
pub enum ScanError {
    UnreadableDir { path: PathBuf, source: io::Error },
}

impl std::fmt::Display for ScanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScanError::UnreadableDir { path, source } => {
                write!(f, "scan: cannot read service dir {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for ScanError {}

/// 扫一遍服务目录（非递归），产出 DLL 条目。
///
/// - 目录不存在 → `Ok(vec![])`；
/// - 条目名 = 文件 stem（unix 的 `lib` 前缀剥离）；
/// - 非服务产物扩展名的文件跳过。
pub fn scan(dir: &Path) -> Result<Vec<ServiceDescriptor>, ScanError> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let rd = std::fs::read_dir(dir).map_err(|source| ScanError::UnreadableDir {
        path: dir.to_path_buf(),
        source,
    })?;

    let mut out = Vec::new();
    for entry in rd {
        let entry = match entry {
            Ok(e) => e,
            // 单个条目读取失败（悬空符号链接等）不致命，跳过
            Err(_) => continue,
        };
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = match path.extension().and_then(|e| e.to_str()) {
            Some(e) if PLUGIN_EXTENSIONS.contains(&e) => e,
            _ => continue,
        };
        // libfoo.dylib → foo (unix cdylib 默认带 lib 前缀)
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let stem = stem.strip_prefix("lib").filter(|s| !s.is_empty()).unwrap_or(stem);
        if stem.is_empty() {
            continue;
        }
        let _ = ext; // 扩展名仅作过滤, 平台差异由编译目标决定
        out.push(ServiceDescriptor::dll(stem.to_string(), path));
    }
    // 确定性顺序（平台 path 排序），reload diff 可预测
    out.sort_by(|a, b| a.name.as_str().cmp(b.name.as_str()));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    /// 每测试独立临时目录（进程 id 隔离, 形状沿用历史 testkit）。
    pub fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "smf-scan-{tag}-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn scan_lists_plugin_artifacts_with_names() {
        let dir = temp_root("list");
        std::fs::write(dir.join("libdemo.dylib"), b"").unwrap();
        std::fs::write(dir.join("helper.so"), b"").unwrap();
        std::fs::write(dir.join("winplug.dll"), b"").unwrap();

        let found = scan(&dir).expect("scan ok");
        let names: Vec<_> = found.iter().map(|d| d.name.as_str().to_string()).collect();
        assert_eq!(names, vec!["demo", "helper", "winplug"], "lib 前缀剥离 + 排序");

        match &found[0].origin {
            crate::catalog::ServiceOrigin::Dll(p) => {
                assert_eq!(p, &dir.join("libdemo.dylib"));
            }
            other => panic!("origin 不对: {other:?}"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn scan_skips_non_plugin_files_and_dirs() {
        let dir = temp_root("skip");
        std::fs::write(dir.join("README.md"), b"").unwrap();
        std::fs::write(dir.join("notes.txt"), b"").unwrap();
        std::fs::create_dir(dir.join("libsub.dylib")).unwrap(); // 目录伪装产物
        let found = scan(&dir).unwrap();
        assert!(found.is_empty(), "非产物跳过、目录跳过, 得到 {:?}", found.iter().map(|d| d.name.as_str()).collect::<Vec<_>>());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_dir_is_empty_not_fatal() {
        let dir = std::env::temp_dir().join("smf-scan-definitely-missing");
        assert!(scan(&dir).unwrap().is_empty());
    }
}
