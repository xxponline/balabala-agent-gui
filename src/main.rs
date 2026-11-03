//! eframe 入口: 只负责开窗口, 真正的逻辑在 `app.rs`。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod agent_entities;
mod app;
mod ui;

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
        Box::new(|_cc| Ok(Box::new(TerminalApp::new()))),
    )
}
