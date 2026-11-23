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
use gpui_kit::{Menu, MenuItem};

/// 当前应用菜单 (task 6.1: Edit/Copy; 6.2 追加 cfg 门控的 Debug 子菜单)。
pub fn build() -> Vec<Menu> {
    // `mut` 供下方 cfg 块 push; release 下块剥离 → 闲置
    // (app.rs legacy 播种的 `#[allow(unused_mut)]` 同款豁免)
    #[allow(unused_mut)]
    let mut menus = vec![Menu::new("Edit").items([MenuItem::action("Copy", TerminalCopy)])];
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

/// 装配期安装: set_menus (task 6.1) + Debug 菜单项 action 分发接线 (task 6.3)。
///
/// 6.3 链路: 菜单项点击 → 平台 `on_app_menu_action` → `cx.dispatch_action`
/// → 活动窗口派发树的**全局 Bubble 段** → 本监听 → 5.1
/// [`TerminalApp::open_debug_section`]。监听体内**必须 `cx.defer`**:
/// 菜单 action 派发运行在窗口 update lease 内 (`update_window_erased`
/// 期间窗口被 `Option::take`), 同步嵌套 `handle.update` 拿回 None 必败
/// (`weak.update` 重入 panic 同类教训, `notify_panels` 文档同款);
/// `App::defer` 只在最外层 update finish 时 flush — 届时窗口已归还。
pub fn install(cx: &mut gpui_kit::Context<TerminalApp>) {
    cx.set_menus(build());
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
}

#[cfg(test)]
#[path = "menu_tests.rs"]
mod tests;
