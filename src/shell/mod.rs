//! 外壳装配：DockArea 三槽（左 sidebar | 中心终端 | 右占位）。
//!
//! design D1（采用 gpui-component DockArea）/ D5（左 dock 不可关）/
//! D8（chrome 全 title_bar=false）；spike §10.5 关键实证：
//! - **注册必须 `panel_handle` 包装**（`add_panel_view` / `panel_view`）——
//!   裸 entity 绕过 `title_bar()=false` 判断，每槽多画 30px 标题栏，
//!   压住 spec 置顶的 `+ new workspace`；
//! - 顺序：先 add（按需建 dock）再 `set_dock_collapsible`（对不存在的
//!   dock 是 no-op）。
//!
//! R3 外壳：`TerminalApp::render` 的外层容器承载本模块产出的 DockArea，
//! 未来 titlebar/状态栏/覆盖层挂同一容器。

pub mod placeholder;

use gpui_kit::component::dock::{
    panel_handle, DockArea, DockLayout, DockPlacement, DockSkin,
};
use gpui_kit::*;

use crate::app::TerminalApp;
use crate::pane::TerminalPanel;
use crate::sidebar::SidebarPanel;
use placeholder::RightPlaceholderPanel;

/// `shell::build` 的产出；TerminalApp 保存实体句柄
/// （sidebar 用于 notify 转发，terminal 用于一步切焦点/关闭右 dock 后回焦点）。
pub struct ShellParts {
    pub dock_area: Entity<DockArea>,
    pub sidebar: Entity<SidebarPanel>,
    pub terminal: Entity<TerminalPanel>,
}

/// 在 `TerminalApp::new(window, cx)` 内调用。
///
/// DockSkin 的 Rc 由 DockArea 持有（`with_renderer`），此处不保留。
pub fn build(
    weak_app: WeakEntity<TerminalApp>,
    window: &mut Window,
    cx: &mut Context<TerminalApp>,
) -> ShellParts {
    let (dock_area, _skin) = DockSkin::dock_area("main", Some(1), window, cx);
    // Some(1) = 持久化 schema 版本（D6: 本轮不 load/dump, 先占位）

    let sidebar = cx.new(|cx| SidebarPanel::new(weak_app.clone(), cx));
    let terminal = cx.new(|cx| TerminalPanel::new(weak_app.clone(), cx));
    let right = cx.new(|cx| RightPlaceholderPanel::new(weak_app, cx));

    dock_area.update(cx, |area, cx| {
        // 必须 panel_handle 包装（spike §10.5）；先 add 建 dock,
        // 再 set_collapsible（对不存在的 dock 是 no-op）
        area.add_panel_view(
            panel_handle(sidebar.clone()),
            DockPlacement::Left,
            Some(px(240.)), // = 现 sidebar 宽 (design D7 播种)
            window,
            cx,
        );
        area.add_panel_view(
            panel_handle(right),
            DockPlacement::Right,
            Some(px(240.)),
            window,
            cx,
        );
        // D5: 左 dock 是 spec 固定家具（+ new workspace / [+ agent] 必须可达）
        area.set_dock_collapsible(DockPlacement::Left, false, window, cx);
        // R5: 中心主面板 = DockLayout 单 pane（未来分屏用 h_split()）
        area.set_center(
            DockLayout::tabs().panel_view(panel_handle(terminal.clone()), cx),
            window,
            cx,
        );
    });

    ShellParts {
        dock_area,
        sidebar,
        terminal,
    }
}
