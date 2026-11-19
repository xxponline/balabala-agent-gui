//! 构建指纹（design D8 / D12 路线甲防线）：cdylib 产物在装载前须与宿主指纹
//! 一致，不匹配**显式拒载**——把 TypeId 撕裂从 downcast 静默失败变成可见错误。
//!
//! 指纹 = `rustc 版本` + `契约源树哈希`。诚实说明：rustc 元数据哈希（SVH）
//! 无法从构建系统外部提取，v1 以"契约源树 FNV-1a + rustc 版本"作代理——
//! 契约 crate 零依赖、零 feature（D2），同源树 + 同 rustc ⇒ 元数据一致，
//! TypeId 相等成立。宿主与插件两侧调用**同一份实现**（经 build 脚本），保证可比。

use std::io;
use std::path::{Path, PathBuf};

/// 指纹前缀（格式版本，便于将来换算法）。
pub const FINGERPRINT_PREFIX: &str = "smf-fp1";

/// FNV-1a（std-only，build 脚本可用，无外部依赖）。
pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// 递归哈希契约源树：条目按相对路径排序，混合 `relpath \0 contents`。
pub fn hash_contract_src(dir: &Path) -> io::Result<u64> {
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    collect(dir, dir, &mut entries)?;
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let mut h: u64 = 0xcbf29ce484222325;
    for (rel, content) in entries {
        h ^= fnv1a(rel.as_bytes());
        h = h.wrapping_mul(0x100000001b3);
        h ^= fnv1a(&content);
        h = h.wrapping_mul(0x100000001b3);
    }
    Ok(h)
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) -> io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect(root, &path, out)?;
        } else if path.extension().is_some_and(|e| e == "rs") {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .into_owned();
            out.push((rel, std::fs::read(&path)?));
        }
    }
    Ok(())
}

/// 从某个 crate 的 manifest 目录向上找 workspace 根（存在
/// `crates/contracts/src` 即认），返回契约源目录。
///
/// v1 布局前提（D11 工程结构）：宿主 `crates/service-manager`、插件
/// `plugins/<name>`、契约 `crates/contracts` 同仓；拆独立仓库时此处调参。
pub fn contract_src_from(manifest_dir: &Path) -> Option<PathBuf> {
    let mut dir = Some(manifest_dir);
    while let Some(d) = dir {
        let candidate = d.join("crates").join("contracts").join("src");
        if candidate.is_dir() {
            return Some(candidate);
        }
        dir = d.parent();
    }
    None
}

/// 当前构建所用 rustc 版本（`$RUSTC --version`；build 脚本环境提供）。
pub fn rustc_version() -> io::Result<String> {
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    let out = std::process::Command::new(rustc).arg("--version").output()?;
    if !out.status.success() {
        return Err(io::Error::other("rustc --version failed"));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// 计算指纹。`contract_src = None` → 源树哈希 0（两侧同样处理，仍可比）。
pub fn compute(rustc_version: &str, contract_src: Option<&Path>) -> io::Result<String> {
    let hash = match contract_src {
        Some(dir) => hash_contract_src(dir)?,
        None => 0,
    };
    Ok(format!("{FINGERPRINT_PREFIX}|rustc:{rustc_version}|contracts:{hash:016x}"))
}

/// 装载前校验：一致 → `Ok`；不一致 → [`FingerprintError`]（Display 含
/// "mismatch"，显式拒载而非静默，D8/D12）。
pub fn verify(plugin_fingerprint: &str, host_fingerprint: &str) -> Result<(), FingerprintError> {
    if plugin_fingerprint == host_fingerprint {
        Ok(())
    } else {
        Err(FingerprintError {
            plugin: plugin_fingerprint.to_string(),
            host: host_fingerprint.to_string(),
        })
    }
}

/// 指纹不匹配（拒载错误；调用方据此把条目落 error / 扫描警告，D8）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FingerprintError {
    pub plugin: String,
    pub host: String,
}

impl fmt::Display for FingerprintError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "fingerprint mismatch: plugin={} host={}",
            self.plugin, self.host
        )
    }
}

impl std::error::Error for FingerprintError {}

use std::fmt;

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_contract(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("smf-fp-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("lib.rs"), "pub struct A;").unwrap();
        std::fs::write(dir.join("sub").join("mod.rs"), "pub struct B;").unwrap();
        dir
    }

    #[test]
    fn compute_is_deterministic_and_content_sensitive() {
        let dir = temp_contract("det");
        let a = compute("rustc 1.85.0", Some(&dir)).unwrap();
        let b = compute("rustc 1.85.0", Some(&dir)).unwrap();
        assert_eq!(a, b, "同输入同指纹");

        std::fs::write(dir.join("lib.rs"), "pub struct A; // changed").unwrap();
        let c = compute("rustc 1.85.0", Some(&dir)).unwrap();
        assert_ne!(a, c, "契约变更 → 指纹变化");

        let d = compute("rustc 1.86.0", Some(&dir)).unwrap();
        assert_ne!(c, d, "rustc 版本变化 → 指纹变化");
        assert!(a.starts_with(FINGERPRINT_PREFIX));
        assert!(a.contains("rustc 1.85.0"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn verify_accepts_equal_and_rejects_tampered_with_mismatch_text() {
        // 一致通过
        let fp = "smf-fp1|rustc:x|contracts:0000000000000000";
        verify(fp, fp).expect("指纹一致应通过");

        // 篡改 → 拒, 报错信息含 mismatch
        let tampered = "smf-fp1|rustc:x|contracts:deadbeefdeadbeef";
        let err = verify(tampered, fp).expect_err("篡改指纹应被拒");
        let text = err.to_string();
        assert!(text.contains("mismatch"), "报错信息须含 mismatch: {text}");
        assert!(text.contains(tampered), "须指名插件指纹: {text}");
        assert!(text.contains(fp), "须指名宿主指纹: {text}");
    }

    #[test]
    fn contract_src_lookup_walks_up() {
        let root = temp_contract("lookup");
        // 布局: <root>/crates/contracts/src + <root>/crates/service-manager
        let contracts_src = root.join("crates").join("contracts").join("src");
        std::fs::create_dir_all(&contracts_src).unwrap();
        std::fs::write(contracts_src.join("lib.rs"), "pub struct Shared;").unwrap();
        let nested = root.join("crates").join("service-manager");
        std::fs::create_dir_all(&nested).unwrap();
        let found = contract_src_from(&nested).expect("应向上找到 contracts/src");
        assert_eq!(found, contracts_src);
        std::fs::remove_dir_all(&root).ok();
    }
}
