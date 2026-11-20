//! 外壳装配：DockArea 三槽（左 sidebar | 中心终端 | 右文件树）。
//!
//! design D1（采用 gpui-component DockArea）/ D5（左 dock 不可关）/
//! D8（左右槽 title_bar=false; 中槽 console title_bar=true — 2026-11-10
//! 验收追加顶栏常驻, 方案 B）；spike §10.5 关键实证：
//! - **注册必须 `panel_handle` 包装**（`add_panel_view` / `panel_view`）——
//!   裸 entity 绕过 title_bar 判断, 每槽多画 30px 标题栏，
//!   压住 spec 置顶的 `+ new workspace`；
//! - 顺序：先 add（按需建 dock）再 `set_dock_collapsible`（对不存在的
//!   dock 是 no-op）。
//!
//! R3 外壳：`TerminalApp::render` 的外层容器承载本模块产出的 DockArea，
//! 未来 titlebar/状态栏/覆盖层挂同一容器。

use gpui_kit::component::dock::{panel_handle, DockArea, DockLayout, DockPlacement, DockSkin};
use gpui_kit::*;

use crate::app::TerminalApp;
use crate::sidebar::SidebarPanel;
use agent_service::terminal::{AgentTerminal, TerminalBus};
use std::sync::Arc;
use crate::tree::TreePanel;

/// AgentTerminal 出站窄缝的 GUI 实现 (change agent-service design D7):
/// 面板语义 → TerminalApp 控制方法 (ask/tell 由 app 内部走服务消息)。
struct AppTerminalBus {
    app: WeakEntity<TerminalApp>,
}

impl AppTerminalBus {
    fn new(app: WeakEntity<TerminalApp>) -> Arc<Self> {
        Arc::new(Self { app })
    }
}

impl TerminalBus for AppTerminalBus {
    fn send_key(&self, cx: &mut App, key: crossterm::event::KeyEvent) {
        self.app
            .update(cx, |app, cx| app.send_key(key, cx))
            .ok();
    }

    fn send_text(&self, cx: &mut App, text: &str) {
        self.app
            .update(cx, |app, cx| app.send_text(text, cx))
            .ok();
    }

    fn send_wheel(&self, cx: &mut App, lines: i16) {
        self.app
            .update(cx, |app, cx| app.send_wheel(lines, cx))
            .ok();
    }

    fn send_click(&self, cx: &mut App, x: u16, y: u16) {
        self.app
            .update(cx, |app, cx| app.send_click(x, y, cx))
            .ok();
    }

    fn propagate_grid_size(
        &self,
        cx: &mut App,
        cols: u16,
        rows: u16,
        w: f32,
        h: f32,
        cell_w: gpui_kit::Pixels,
        line_h: gpui_kit::Pixels,
    ) {
        self.app
            .update(cx, |app, cx| {
                app.propagate_grid_size(cols, rows, w, h, cell_w, line_h, cx)
            })
            .ok();
    }
}

/// `shell::build` 的产出；TerminalApp 保存实体句柄
/// （sidebar 用于 notify 转发，terminal 用于一步切焦点/关闭右 dock 后回焦点）。
pub struct ShellParts {
    pub dock_area: Entity<DockArea>,
    pub sidebar: Entity<SidebarPanel>,
    pub terminal: Entity<AgentTerminal>,
    /// 文件树面板 (notify 转发名单 + 一步焦点目标; app 持实体)
    pub tree: Entity<TreePanel>,
}

/// 在 `TerminalApp::new(window, cx)` 内调用。
///
/// DockSkin 的 Rc 由 DockArea 持有（`with_renderer`），此处不保留。
pub fn build(
    weak_app: WeakEntity<TerminalApp>,
    window: &mut Window,
    cx: &mut Context<TerminalApp>,
) -> ShellParts {
    let (dock_area, skin) = DockSkin::dock_area("main", Some(1), window, cx);
    // Some(1) = 持久化 schema 版本 (D6: 本轮不 load/dump, 先占位)
    // 关闭按钮默认关 (gpui-component: close_button_visible=false) —
    // 多签后 viewer 签必须可关 (spec: 签关闭与去重表生命周期); gate
    // 仍是 `closable`, console (closable=false) 不会出 ×。
    skin.set_close_button_visible(true, cx);

    let sidebar = cx.new(|cx| SidebarPanel::new(weak_app.clone(), cx));
    let terminal = cx.new(|cx| AgentTerminal::new(AppTerminalBus::new(weak_app.clone()), cx));
    let right = cx.new(|cx| TreePanel::new(weak_app.clone(), cx));

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
            panel_handle(right.clone()),
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
        tree: right,
    }
}
