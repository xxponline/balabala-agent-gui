//! gpui 入口: 只负责开窗口, 真正的逻辑在 `app.rs`。
//!
//! UI 栈为 gpui-kit (change: port-ui-to-gpui-kit); 迁移前的旧 UI
//! 实现已随本 change 移除 (见 git history)。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
// 门控在模块文件内 (#![cfg(debug_assertions)]): release 下模块为空壳,
// 注册表/section 代码不进二进制 (design D5)
mod debug;

mod menu;
mod project_model;
mod service_layer;
mod solution_config;
mod property_detail_manager;
mod shell;
mod solution_overview;
mod ui;
mod workspace;
// 启动关卡 (launcher-startup): 启动阶段唯一窗口, 先于主窗
mod launcher;

fn main() {
    // Windows GUI 子系统下 (release, 无控制台) crossterm 的 ANSI 支持探测修复:
    // `supports_ansi()` 先试 enable_vt_processing (需有效控制台句柄, GUI 进程必败),
    // 失败则回退看 TERM != "dumb"。Windows 默认不设 TERM → 返回 false →
    // crossterm queue() 改走 execute_winapi() 直连控制台句柄 → Os 6 (handle
    // invalid) → pi-shell-rs embed 事件线程首帧 draw 即退 → 会话秒退、pi 子进程
    // 被杀 (实测: release 弹控制台窗口后立即结束; debug 是控制台子系统故不触发)。
    // 必须在任何 crossterm queue 调用前 (supports_ansi 的 Once 初始化前) 设置。
    #[cfg(windows)]
    unsafe {
        if std::env::var_os("TERM").is_none() {
            std::env::set_var("TERM", "xterm-256color");
        }
    }

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

        // 启动关卡 (launcher-startup, design D1): 只出 Launcher。主窗在
        // 用户选定 workspace 后才由 Launcher 交棒打开 — 自动恢复已退场。
        launcher::open(cx);
    });
}
