//! 应用菜单装配 (add-debug-console, specs/app-menu-bar)。
//!
//! design D4: OS 原生优先 — `cx.set_menus` 交平台原生渲染 (macOS 真系统
//! 菜单栏, 快捷键提示从 keymap 自动带出); Win/Linux 第一版同样先试,
//! 实测不理想再上窗口内 AppMenuBar 兜底 (task 6.4)。
//! design D6: 菜单栏常驻 — Edit/Copy 在 release 也产出; Debug 子菜单
//! 才受 `cfg(debug_assertions)` 门控 (task 6.2)。
//! design D11: 装配完成 (`TerminalApp::new_inner` 尾部) 构建一次;
//! 注册表变化整根 `set_menus` 重推。

use agent_service::terminal::TerminalCopy;
use crate::app::TerminalApp;
use crate::theme_preference::{SetThemeMode, ThemePreference};
use gpui_kit::component::menu::AppMenuBar;
use gpui_kit::{Entity, Menu, MenuItem};

/// Launcher 期的菜单 (launcher-startup 5.1, design D5)。
///
/// 只有 `Edit` — `Debug` 子菜单的 action 监听要 `cx.weak_entity::<
/// TerminalApp>()` (`install` 内), 而 Launcher 阶段主窗尚未构造。
/// 主窗装配完成时会整根 `set_menus` 重推带 Debug 的完整版
/// (spec: Menu content rebuild on change)。
pub fn build_launcher() -> Vec<Menu> {
    vec![Menu::new("Edit").items([MenuItem::action("Copy", TerminalCopy)])]
}

/// 当前应用菜单 (task 6.1: Edit/Copy; 6.2 追加 cfg 门控的 Debug 子菜单;
/// add-theme-preference 3.1: View > Theme 三选一)。
///
/// `copy_enabled` 只驱动 Copy 项的**静态**灰显位 (design D2: 灰显只看选区,
/// 焦点不参与) —— 窗口内 `AppMenuBar` → `PopupMenu` 直接拷 `disabled`,
/// 它没有 macOS 那样的 `on_validate_app_menu_command` 动态判定。
///
/// `theme_preference` 驱动 View > Theme 的勾选位 (spec: exactly one
/// check-marked): 纯函数快照, 切换后由 [`TerminalApp::rebuild_menus`]
/// 重算 (spec: rebuild preserves the check mark)。
pub fn build(copy_enabled: bool, theme_preference: ThemePreference) -> Vec<Menu> {
    // `mut` 供下方 cfg 块 push; release 下块剥离 → 闲置
    // (app.rs legacy 播种的 `#[allow(unused_mut)]` 同款豁免)
    #[allow(unused_mut)]
    let mut menus = vec![Menu::new("Edit").items([
        MenuItem::action("Copy", TerminalCopy).disabled(!copy_enabled),
    ])];
    // View > Theme 三选一 (add-theme-preference 3.1, specs/app-menu-bar:
    // View menu carries theme selection)。子菜单路径 View > Theme,
    // 项序固定 Follow System / Light / Dark, 恰一项勾选。
    menus.push(
        Menu::new("View").items([MenuItem::submenu(Menu::new("Theme").items([
            MenuItem::action(
                "Follow System",
                SetThemeMode {
                    mode: ThemePreference::System,
                },
            )
            .checked(theme_preference == ThemePreference::System),
            MenuItem::action(
                "Light",
                SetThemeMode {
                    mode: ThemePreference::Light,
                },
            )
            .checked(theme_preference == ThemePreference::Light),
            MenuItem::action(
                "Dark",
                SetThemeMode {
                    mode: ThemePreference::Dark,
                },
            )
            .checked(theme_preference == ThemePreference::Dark),
        ]))]),
    );
    // Debug 子菜单整块仅 debug 构建产出 (task 6.2, design D5): 数据源
    // `sections()` 与项 action `OpenDebugSection` 都在 `crate::debug` 的
    // `#![cfg(debug_assertions)]` 门控内 — release 下本块不编译,
    // 菜单名/节标题/action 名字符串均不进二进制 (字符串验收)。
    #[cfg(debug_assertions)]
    {
        let items = crate::debug::sections()
            .into_iter()
            .map(|s| {
                MenuItem::action(
                    s.title,
                    crate::debug::OpenDebugSection {
                        id: s.id.to_string(),
                    },
                )
            })
            .collect::<Vec<_>>();
        menus.push(Menu::new("Debug").items(items));
    }
    menus
}

/// design D3 第一步: 把 `build()` 的当前内容喂进 `GlobalState` —— 窗口内
/// [`AppMenuBar::reload()`](gpui_kit::component::menu::AppMenuBar) 的唯一
/// 数据源。[`install`](install) 与 `TerminalApp::rebuild_menus` **共用本函数**,
/// 保证喂食口径单点 (两处各写一遍 = 必然漂移)。
///
/// `Menu` 非 Clone, 每次重新 `build()` (读 debug 注册表 + 分配, 廉价)。
///
/// `copy_enabled` 透传给 [`build`] (design D3 的灰显来源, task 3.2);
/// `theme_preference` 透传给 [`build`] (add-theme-preference 3.1 的勾选来源)。
pub fn feed_global(
    cx: &mut gpui_kit::App,
    copy_enabled: bool,
    theme_preference: ThemePreference,
) {
    gpui_kit::base::GlobalState::global_mut(cx)
        .set_app_menus(
            build(copy_enabled, theme_preference)
                .into_iter()
                .map(|m| m.owned())
                .collect(),
        );
}

/// 装配期安装: set_menus (task 6.1) + 窗口内菜单栏 (win-in-window-app-menu-bar
/// design D1) + Debug 菜单项 action 分发接线 (task 6.3)。
///
/// 返回窗口内 [`AppMenuBar`] 实体: 非 mac 平台为 `Some` (macOS 走原生
/// `set_menus`, design D6 — 挂载会双份, 故不建实例), macOS 恒 `None`。
///
/// design D1: **必须先** `set_app_menus` **再**建 `AppMenuBar` ——
/// `AppMenuBar::new()` 内部立刻 `reload()` 读 `GlobalState::app_menus()`,
/// 顺序反了首帧是一条空菜单栏且无任何报错。D6: 喂数据不做平台门控
/// (macOS 侧数据保持单源)。`Menu` 非 Clone, `build()` 需调两次。
///
/// 6.3 链路: 菜单项点击 → 平台 `on_app_menu_action` → `cx.dispatch_action`
/// → 活动窗口派发树的**全局 Bubble 段** → 本监听 → 5.1
/// [`TerminalApp::open_debug_section`]。监听体内**必须 `cx.defer`**:
/// 菜单 action 派发运行在窗口 update lease 内 (`update_window_erased`
/// 期间窗口被 `Option::take`), 同步嵌套 `handle.update` 拿回 None 必败
/// (`weak.update` 重入 panic 同类教训, `notify_panels` 文档同款);
/// `App::defer` 只在最外层 update finish 时 flush — 届时窗口已归还。
pub fn install(
    cx: &mut gpui_kit::Context<TerminalApp>,
    copy_enabled: bool,
    theme_preference: ThemePreference,
) -> Option<Entity<AppMenuBar>> {
    cx.set_menus(build(copy_enabled, theme_preference));
    // design D1 前半: 先把菜单喂进 GlobalState (窗口内 AppMenuBar 的数据源)
    feed_global(cx, copy_enabled, theme_preference);
    // design D1 后半 + D6: 实例只为挂载而建 → 只在非 mac 建 (macOS 挂原生,
    // 窗口内再挂一条会双份); macOS 恒 None, 字段与挂载两侧同 cfg 语义
    #[cfg(not(target_os = "macos"))]
    let menu_bar = Some(gpui_kit::component::menu::AppMenuBar::new(cx));
    #[cfg(target_os = "macos")]
    let menu_bar = None;
    #[cfg(debug_assertions)]
    {
        let weak = cx.weak_entity();
        // 走 `App::on_action` **全局监听** (任务文本链路): `Context` 也有同名
        // 固有方法 (窗口级, 签名 (TypeId, &mut Window, f)) 会遮蔽方法解析 —
        // 经 `&mut *cx` 提升到 App 路径。全局监听存于 App 的
        // `global_action_listeners` — 菜单派发的全局 Bubble 段必查 (design
        // §spike: 焦点窗口 action 树或全局监听)。
        let app: &mut gpui_kit::App = cx;
        app.on_action::<crate::debug::OpenDebugSection>(move |action, cx| {
            let Some(handle) = cx.active_window() else { return };
            let id = action.id.clone();
            let weak = weak.clone(); // 监听是 Fn, 每次调用 clone 句柄
            // 必须 defer (notify_panels 同款): 菜单 action 派发运行在窗口
            // update lease 内, 同步 handle.update 同窗口必败 (负向对照实测:
            // 拿回 None → 开签不执行); defer 只在最外层 update finish 时
            // flush — 届时窗口已归还。
            cx.defer(move |cx| {
                let _ = handle.update(cx, |_, window, cx| {
                    let _ = weak.update(cx, |app, cx| {
                        app.open_debug_section(&id, window, cx);
                    });
                });
            });
        });
    }
    // 主题切换 (add-theme-preference 3.2): View > Theme 点击 → defer
    // (菜单 action 派发在窗口 update lease 内, 同步嵌套实体更新会
    // 重入 — Debug 监听同款教训) → 实体五步链路。
    {
        let weak = cx.weak_entity();
        let app: &mut gpui_kit::App = cx;
        app.on_action::<SetThemeMode>(move |action, cx| {
            let Some(handle) = cx.active_window() else { return };
            let mode = action.mode;
            let weak = weak.clone(); // 监听是 Fn, 每次调用 clone 句柄
            cx.defer(move |cx| {
                let _ = handle.update(cx, |_, window, cx| {
                    let _ = weak.update(cx, |app, cx| {
                        app.set_theme_mode(mode, window, cx);
                    });
                });
            });
        });
    }
    menu_bar
}

#[cfg(test)]
#[path = "menu_tests.rs"]
mod tests;
