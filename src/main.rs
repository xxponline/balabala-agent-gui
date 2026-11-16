//! gpui 入口: 只负责开窗口, 真正的逻辑在 `app.rs`。
//!
//! UI 栈为 gpui-kit (change: port-ui-to-gpui-kit); 迁移前的旧 UI
//! 实现已随本 change 移除 (见 git history)。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod agent_entities;
mod app;
mod pane;
mod property_detail_manager;
mod shell;
mod sidebar;
mod tree;
mod ui;
mod workspace;
mod workspace_property_source;

use app::TerminalApp;
use gpui_kit::AppContext;

fn main() {
    gpui_kit::application()
        // 图标资产 (tab 关闭 × 等 SVG): gpui 的默认 AssetSource 是 `()`,
        // 不挂 gpui_kit::assets::Assets 则所有 svg 加载失败 (空白按钮, 2026-11-10
        // headed 验收实测)
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx| {
        gpui_kit::init(cx);
        // 关窗即退: gpui 的 Default 在 macOS 上是 Explicit (实测关窗
        // 进程残留); LastWindowClosed 与 eframe 原行为一致, 否则
        // drain 循环与 pty 子进程会随僵尸进程残留。
        cx.set_quit_mode(gpui_kit::QuitMode::LastWindowClosed);
        // 主题跟随系统外观 (add-light-dark-theme D1): 不再强制 Light —
        // gpui-component 默认 Light, TerminalApp::new 在窗口启动时
        // sync_system_appearance 同步到系统外观并订阅后续热切换。

        let options = gpui_kit::WindowOptions {
            window_bounds: Some(gpui_kit::WindowBounds::centered(
                gpui_kit::size(gpui_kit::px(1000.), gpui_kit::px(600.)),
                cx,
            )),
            titlebar: Some(gpui_kit::TitlebarOptions {
                title: Some("pi agent shell".into()),
                ..Default::default()
            }),
            ..Default::default()
        };

        gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| {
                let mut app = match TerminalApp::new(window, cx) {
                    Ok(app) => app,
                    Err(e) => {
                        rfd::MessageDialog::new()
                            .set_title("Failed to load workspaces")
                            .set_description(&e)
                            .set_level(rfd::MessageLevel::Error)
                            .show();
                        eprintln!("failed to load workspaces: {e}");
                        std::process::exit(1);
                    }
                };
                app.start_update_loop(cx);
                app
            })
        })
        .expect("failed to open window");
    });
}
