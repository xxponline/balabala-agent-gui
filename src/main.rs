//! eframe 入口: 只负责开窗口, 真正的逻辑在 `app.rs`。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod agent_entities;
mod app;
mod ui;
mod workspace;

use app::TerminalApp;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size((1000.0, 600.0))
            .with_title("pi agent shell"),
        ..Default::default()
    };

    eframe::run_native(
        "pi-agent-shell",
        options,
        Box::new(|_cc| {
            let app = match TerminalApp::new() {
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
            Ok(Box::new(app))
        }),
    )
}
