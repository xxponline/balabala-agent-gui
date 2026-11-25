//! 主题偏好 (add-theme-preference, design D1/D2): 三态 system/light/dark。
//!
//! 全局便利数据: 存 `<ProjectDirs>/theme_preference.json`, 与
//! `workspace_history.json` 同目录、同语义 —— 缺失/损坏/版本不符/未知值
//! 一律回落默认 `System` (spec: Missing or unreadable preference falls back
//! to the default), 不致命。三态建模在我方 (gpui-component 的 `ThemeMode`
//! 只有 Light/Dark, design D1), `System` 在边缘经
//! [`ThemePreference::resolve`] 一次性解析为 `ThemeMode`。
//!
//! 写纪律与 `workspace/history.rs` 同款: `cfg(test)` 的 `default_path` 是
//! **进程独享**临时路径 —— 会写文件的测试必须注入自己的 temp 路径,
//! 默认路径只服务"无文件 → 默认值"前提 (同进程共享, 不用于断言写入)。

use std::path::{Path, PathBuf};

use gpui_kit::WindowAppearance;
use gpui_kit::component::theme::ThemeMode;

/// 偏好文件格式版本 (对齐 `History::version` 的纪律: 不认识 → 默认值)。
pub const PREFERENCE_VERSION: u32 = 1;

/// 三态主题偏好。`System` 为默认: 无文件/坏文件即此值。
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum ThemePreference {
    /// 跟随系统外观 (默认)。
    #[default]
    System,
    /// 钉住浅色 (系统外观变化不影响)。
    Light,
    /// 钉住深色 (系统外观变化不影响)。
    Dark,
}

impl ThemePreference {
    /// 解析为实际渲染模式 (design D1): `System` → 当前外观, pinned → 自身。
    ///
    /// 外观来源由调用方决定: 启动期无窗用 `cx.window_appearance()`,
    /// 有窗时上游建议 `window.appearance()` (Linux 更准, 见
    /// `Theme::sync_system_appearance` 注释)。
    pub fn resolve(self, appearance: WindowAppearance) -> ThemeMode {
        match self {
            ThemePreference::System => ThemeMode::from(appearance),
            ThemePreference::Light => ThemeMode::Light,
            ThemePreference::Dark => ThemeMode::Dark,
        }
    }
}

/// 按偏好 apply 主题 (design D3): pinned → `Theme::change` (不看外观),
/// System → `sync_system_appearance` (无窗用 `cx.window_appearance()`,
/// 有窗用 `window.appearance()`, 上游注释: Linux 更准)。
///
/// 幂等 —— `main.rs` 启动 (无窗, 先于 Launcher) 与 `TerminalApp::new`
/// (有窗) 各调一次。
pub fn apply_theme_preference(
    pref: ThemePreference,
    window: Option<&mut gpui_kit::Window>,
    cx: &mut gpui_kit::App,
) {
    match pref {
        ThemePreference::System => {
            gpui_kit::component::theme::Theme::sync_system_appearance(window, cx)
        }
        pinned => {
            // pinned 的 resolve 忽略外观参数 → 恒为自身
            let mode = pinned.resolve(cx.window_appearance());
            gpui_kit::component::theme::Theme::change(mode, None, cx);
        }
    }
}

/// 菜单 View > Theme 三选一的派发动作 (add-theme-preference 3.1)。
/// handler 在 `TerminalApp` (3.2): 更新字段 → apply → 落盘 → 推会话 →
/// `rebuild_menus` (刷勾选)。`no_json`: 纯内存 action, 不进序列化。
#[derive(Clone, PartialEq, gpui_kit::Action)]
#[action(namespace = theme, no_json)]
pub struct SetThemeMode {
    pub mode: ThemePreference,
}

/// 落盘形态 (`version` + `mode`), 结构纪律对齐 `History`。
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct StoredPreference {
    version: u32,
    mode: ThemePreference,
}

/// 读偏好: 缺失/解析失败/版本不符 → 默认 `System` (便利数据语义)。
pub fn load_at(path: &Path) -> ThemePreference {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|c| serde_json::from_str::<StoredPreference>(&c).ok())
        .filter(|s| s.version == PREFERENCE_VERSION)
        .map(|s| s.mode)
        .unwrap_or_default()
}

/// 写偏好: 建父目录 + pretty JSON (失败返回描述, 调用方按便利数据
/// 语义只记日志 —— spec 面向的是偏好内容, 不是写失败的 UI 呈现)。
pub fn save_at(mode: ThemePreference, path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
        }
    }
    let stored = StoredPreference {
        version: PREFERENCE_VERSION,
        mode,
    };
    let json = serde_json::to_string_pretty(&stored).map_err(|e| format!("serialize: {e}"))?;
    std::fs::write(path, json).map_err(|e| format!("write {}: {e}", path.display()))
}

/// 全局偏好文件路径。
///
/// - 正式: 平台应用数据目录 (macOS: `~/Library/Application Support/
///   com.balabala.balabala-agent-gui/`), `directories` crate 解析 ——
///   与 `workspace_history.json` 同目录 (design D2: 全局而非工作区,
///   Launcher 期尚无工作区可读)。
/// - 测试 (`cfg(test)`): 进程独享临时路径 (见模块 doc 的写纪律)。
pub fn default_path() -> PathBuf {
    #[cfg(test)]
    {
        std::env::temp_dir().join(format!(
            "balabala-theme-preference-{}.json",
            std::process::id()
        ))
    }
    #[cfg(not(test))]
    {
        directories::ProjectDirs::from("com.balabala", "", "balabala-agent-gui")
            .map(|d| d.data_dir().join("theme_preference.json"))
            .unwrap_or_else(|| PathBuf::from("theme_preference.json"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::TestAppContext;
    use gpui_kit::component::theme::{Theme, ThemeMode};

    /// 每测独享文件 (pid + 纳秒, history 同款) —— 并行不互踩。
    fn temp_file(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "theme-pref-{name}-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn round_trips_all_three_states() {
        for pref in [
            ThemePreference::System,
            ThemePreference::Light,
            ThemePreference::Dark,
        ] {
            let path = temp_file(&format!("{pref:?}"));
            save_at(pref, &path).expect("save");
            assert_eq!(load_at(&path), pref, "round-trip {pref:?}");
            let _ = std::fs::remove_file(&path);
        }
    }

    #[test]
    fn missing_file_is_system_default() {
        let path = temp_file("missing");
        let _ = std::fs::remove_file(&path); // 保证不存在
        assert_eq!(load_at(&path), ThemePreference::System, "缺失 = 默认");
    }

    #[test]
    fn corrupt_file_is_system_default() {
        let path = temp_file("corrupt");
        std::fs::write(&path, "{ not json").expect("write");
        assert_eq!(load_at(&path), ThemePreference::System, "损坏 = 默认");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn wrong_version_is_system_default() {
        let path = temp_file("version");
        std::fs::write(&path, r#"{"version":99,"mode":"dark"}"#).expect("write");
        assert_eq!(load_at(&path), ThemePreference::System, "版本不符 = 默认");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn unknown_mode_value_is_system_default() {
        let path = temp_file("unknown");
        std::fs::write(&path, r#"{"version":1,"mode":"purple"}"#).expect("write");
        assert_eq!(load_at(&path), ThemePreference::System, "未知值 = 默认");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn resolve_maps_three_states_to_two_state_mode() {
        // System → 按外观; pinned → 自身, 不看外观 (钉住语义)
        assert_eq!(
            ThemePreference::System.resolve(WindowAppearance::Dark),
            ThemeMode::Dark
        );
        assert_eq!(
            ThemePreference::System.resolve(WindowAppearance::Light),
            ThemeMode::Light
        );
        assert_eq!(
            ThemePreference::Light.resolve(WindowAppearance::Dark),
            ThemeMode::Light,
            "钉浅无视深色外观"
        );
        assert_eq!(
            ThemePreference::Dark.resolve(WindowAppearance::Light),
            ThemeMode::Dark,
            "钉深无视浅色外观"
        );
    }

    // ---- 启动 apply 组合 (main.rs 的两行: load_at(default) + apply) ----
    // 用注入路径复现同款组合 —— 默认路径进程共享, 写纪律见模块 doc。

    /// 钉深文件启动: apply 后 mode == Dark。
    /// 自证性: init 强制 Light (gpui-component theme::init), 不 apply 则
    /// 断言必败; 无论测试平台外观如何, 钉住态都应胜出。
    #[gpui_kit::test]
    fn startup_apply_honors_pinned_preference(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let path = temp_file("pinned-dark");
        save_at(ThemePreference::Dark, &path).expect("save");
        let pref = load_at(&path);
        cx.update(|cx| apply_theme_preference(pref, None, cx));
        cx.update(|cx| {
            assert_eq!(
                Theme::global(cx).mode,
                ThemeMode::Dark,
                "钉深文件启动 → 深色, 不跟外观"
            );
        });
        let _ = std::fs::remove_file(&path);
    }

    /// 无文件启动: apply 后 mode == 外观推导值 (spec: Missing or unreadable
    /// preference falls back to the default → System → 跟随)。
    #[gpui_kit::test]
    fn startup_apply_missing_file_follows_appearance(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let path = temp_file("absent");
        let _ = std::fs::remove_file(&path); // 保证不存在
        let pref = load_at(&path);
        assert_eq!(pref, ThemePreference::System, "无文件 = 默认 System");
        cx.update(|cx| apply_theme_preference(pref, None, cx));
        cx.update(|cx| {
            assert_eq!(
                Theme::global(cx).mode,
                ThemeMode::from(cx.window_appearance()),
                "无文件 → System → 跟随外观"
            );
        });
    }
}
