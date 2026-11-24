//! Launcher — 启动关卡窗口 (change: launcher-startup, design D1/D4)。
//!
//! 启动阶段的**唯一窗口**: 读历史 (只读 fs)、渲染列表、选定后校验 marker,
//! 校验通过才交棒给主窗并关掉自己。不可跳过; 关窗即中断启动
//! (`QuitMode::LastWindowClosed` 下它就是最后一个窗口)。
//!
//! **Launcher 存续期间不启动任何其他逻辑** (spec: No other subsystem starts
//! during the Launcher) — 不 bootstrap 服务层、不装配 dock、不装主窗菜单。
//! 它不持有 `TerminalApp`, 两窗不共享状态, 只通过选定的 path 交接 (D1)。
//!
//! **校验分界 (design D4)**: Launcher 做*建议性预检* — 失败就地报错并留在
//! 原地, 进程不退 (关卡不能自己把用户踢出去); 只有校验通过才交棒。主窗的
//! `probe_open` + solution load 是*权威探测*, 那里失败才 `surface_fatal`。
//!
//! 顶层不用 `gpui_kit::*` glob — 会把 `#[test]` 宏遮蔽掉 (app.rs 测试
//! 模块同款教训), 一律显式导入。

use std::path::{Path, PathBuf};

use gpui_kit::base::TestTextExt;
use gpui_kit::component::ActiveTheme;
use gpui_kit::{
    AnyElement, AnyWindowHandle, App, AppContext, AsyncApp, Context, InteractiveElement,
    IntoElement, ParentElement, Render, StatefulInteractiveElement, Styled, Task, TestSupportExt,
    Window, WindowBounds, WindowOptions, div, px, size,
};

use crate::app::TerminalApp;
use crate::project_model::{self, MarkerStatus};
use crate::workspace::history::History;

/// Launcher 窗口实体。
pub struct LauncherView {
    /// most-recent-first 的历史条目 (构造时读一次, 不随外部变化刷新)。
    ///
    /// 读侧所有权归 Launcher: 构造前完成 legacy 播种 (design D2), 写侧
    /// (`commit_open` 的 push) 仍归主窗 — 两窗不共享状态, 只交接 path。
    entries: Vec<PathBuf>,
    /// 就地校验错误 (design D4: 报错留在 Launcher, 不退出)。
    error: Option<String>,
    /// 在途对话框任务 (drop 即取消 — 窗口关闭即取消)。
    _dialog_task: Option<Task<()>>,
}

impl LauncherView {
    /// 构造实体: 读一次历史 (fs, 无服务)。
    pub fn new(history_path: PathBuf, _cx: &mut Context<Self>) -> Self {
        Self {
            entries: History::load_at(&history_path).recent,
            error: None,
            _dialog_task: None,
        }
    }

    /// 历史条目 (most-recent-first) — 渲染与测试的读口。
    pub fn history_entries(&self) -> &[PathBuf] {
        &self.entries
    }

    /// 就地错误的读口 (测试断言用; 渲染直接读字段)。
    #[cfg(test)]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// 选定一个工作区路径 (历史条目 / picker 回填都走这里)。
    ///
    /// design D4 的建议性预检: 目录不存在 / marker 解析失败 → 就地报错并
    /// **留在 Launcher**; marker 缺失 → 确认创建; 合法 → 交棒主窗。
    ///
    /// 收 `AnyWindowHandle` 而非 `&mut Window`: picker 在 async 里回来,
    /// 那里拿不到窗口租约 — 句柄可以在任意上下文取回窗口。
    pub fn choose(
        &mut self,
        path: PathBuf,
        launcher: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        self.error = None;
        if !path.is_dir() {
            self.error = Some(format!("{} does not exist", path.display()));
            cx.notify();
            return;
        }
        match project_model::marker_status(&path) {
            MarkerStatus::Corrupt(msg) => {
                self.error = Some(msg);
                cx.notify();
            }
            MarkerStatus::Missing => self.confirm_create(path, launcher, cx),
            MarkerStatus::Valid(_) => {
                // 交棒放进 defer — 同步移除会在本 update 的 trail 里先
                // 摘掉 Launcher, 早于主窗打开 (design D1)。
                cx.defer(move |app| handoff(path, launcher, app));
            }
        }
    }

    /// 打开目录选择器 → 选定 (spec: Workspace opening via directory picker)。
    ///
    /// 取消 picker = 什么都不发生 (spec: Cancelling … unchanged)。
    /// 这段 rfd 胶水原在 `TerminalApp::request_workspace_open` — 随切换口
    /// 删除归入 Launcher (launcher-startup 4.2)。
    fn pick_and_choose(&mut self, launcher: AnyWindowHandle, cx: &mut Context<Self>) {
        let task = cx.spawn(async move |this, cx: &mut AsyncApp| {
            let Some(handle) = rfd::AsyncFileDialog::new().pick_folder().await else {
                return; // 取消 picker: 不变
            };
            let picked = handle.path().to_path_buf();
            let path = std::fs::canonicalize(&picked).unwrap_or(picked);
            let _ = this.update(cx, |this, cx| this.choose(path, launcher, cx));
        });
        self._dialog_task = Some(task);
    }

    /// marker 缺失 → 确认后创建 (spec: Marker-less directory offers creation)。
    ///
    /// rfd 对话框是正式构建胶水, 无头不触碰 — 决策与副作用全部在
    /// [`Self::on_create_decision`], 这里只负责问 Yes/No。
    fn confirm_create(
        &mut self,
        path: PathBuf,
        launcher: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        let task = cx.spawn(async move |this, cx: &mut AsyncApp| {
            let confirm = rfd::AsyncMessageDialog::new()
                .set_title("Not a workspace")
                .set_description(format!(
                    "{} has no workspace marker. Create a new workspace here?",
                    path.display()
                ))
                .set_level(rfd::MessageLevel::Info)
                .set_buttons(rfd::MessageButtons::YesNo)
                .show()
                .await;
            let yes = matches!(confirm, rfd::MessageDialogResult::Yes);
            let _ = this.update(cx, |this, cx| {
                this.on_create_decision(yes, path, &launcher, cx)
            });
        });
        self._dialog_task = Some(task);
    }

    /// 创建决策的执行体 (rfd 之外的全部副作用 — 无头可测)。
    ///
    /// - 拒绝 → 直接返回: 留在 Launcher, 无 workspace, 无文件 (spec:
    ///   declining leaves the Launcher open with no workspace)
    /// - 确认 → 落盘两文件; 成功则交棒, 失败就地报错 (红条契约)
    fn on_create_decision(
        &mut self,
        confirmed: bool,
        path: PathBuf,
        launcher: &AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        if !confirmed {
            return; // 拒绝: 留在 Launcher, 什么都不发生
        }
        match project_model::create_workspace_files(&path) {
            Ok(()) => {
                // 交棒顺序同 Valid 路径: 先开主窗, 再关 Launcher (design D1)
                let launcher = *launcher;
                cx.defer(move |app| handoff(path, launcher, app));
            }
            Err(e) => {
                self.error = Some(e);
                cx.notify();
            }
        }
    }
}

/// 启动入口: 播种 → 开 Launcher (main.rs 调用)。
pub fn open(cx: &mut App) -> AnyWindowHandle {
    open_inner(
        cx,
        crate::workspace::history::default_path(),
        crate::workspace::store::legacy_path(),
    )
}

/// 开 Launcher 窗 (指定历史与 legacy 路径) — **hermetic 测试缝**。
///
/// 生产走 [`open`]; 本函数只服务 launcher 测试, 让测试注入自己的临时
/// 历史与 legacy 路径而不碰仓库根/应用数据目录。
///
/// 返回窗口句柄, 供测试断言"唯一窗口就是 Launcher"。
#[cfg(test)]
pub fn open_with_history(
    cx: &mut App,
    history_path: PathBuf,
    legacy_path: PathBuf,
) -> AnyWindowHandle {
    open_inner(cx, history_path, legacy_path)
}

/// 实际开窗: **先播种再呈现** (spec: Seeding SHALL complete before the
/// Launcher presents history) — `LauncherView::new` 一构造就读历史,
/// 播种晚了列表里就看不到 legacy 条目。
fn open_inner(cx: &mut App, history_path: PathBuf, legacy_path: PathBuf) -> AnyWindowHandle {
    seed_history(&history_path, &legacy_path);

    // 菜单栏两步装的第一步 (design D5): Launcher 期至少一个菜单
    // (spec: Application menu bar is always present)。带 Debug 的完整版
    // 由主窗 `menu::install` 整根重推。
    cx.set_menus(crate::menu::build_launcher());

    let options = WindowOptions {
        window_bounds: Some(WindowBounds::centered(size(px(480.), px(560.)), cx)),
        titlebar: Some(gpui_kit::TitlebarOptions {
            title: Some("balabala".into()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let (handle, _entity) = gpui_kit::open_window(options, cx, move |_window, cx| {
        cx.new(|cx| LauncherView::new(history_path, cx))
    })
    .expect("failed to open launcher window");
    handle
}

/// legacy `.workspaces.json` 一次性播种 (design D2, 读侧归 Launcher)。
///
/// 改动才写盘; 播种本身永不改写 legacy 文件 (workspace-management:
/// Legacy migration — the application SHALL NOT rewrite the legacy file)。
fn seed_history(history_path: &Path, legacy_path: &Path) {
    let mut history = History::load_at(history_path);
    if crate::workspace::store::seed_legacy(&mut history, legacy_path)
        && let Err(e) = history.save_at(history_path)
    {
        eprintln!("workspace history save failed: {e}");
    }
}

/// 校验通过后的交棒: 开主窗 → 关 Launcher (design D1 的顺序契约)。
///
/// **必须先开主窗再关 Launcher** — `QuitMode::LastWindowClosed` 下顺序反了
/// 会出现零窗口瞬退。两者同批 (defer) 执行, `remove_window` 只置标志由
/// 同一次 update 的 `trail()` 摘窗, 不会提前于主窗打开。
fn handoff(path: PathBuf, launcher: AnyWindowHandle, cx: &mut App) {
    open_main_window(cx, path);
    let _ = launcher.update(cx, |_, window, _| window.remove_window());
}

/// 打开主窗 (装配 + 装载 Launcher 交棒的 path)。
///
/// `TerminalApp::new` 失败 (服务层 bootstrap 不可达) → 错误框 + 退出 —
/// 与 A1 的 "失败就退出" 一致, 且此时 Launcher 已无法退回。
fn open_main_window(cx: &mut App, path: PathBuf) {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::centered(size(px(1000.), px(600.)), cx)),
        titlebar: Some(gpui_kit::TitlebarOptions {
            title: Some("pi agent shell".into()),
            ..Default::default()
        }),
        ..Default::default()
    };
    gpui_kit::open_window(options, cx, move |window, cx| {
        cx.new(|cx| {
            let mut app = match TerminalApp::new(window, cx, Some(path)) {
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
    .expect("failed to open main window");
}

impl Render for LauncherView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 语义 token 快照 (Hsla: Copy) — 闭包捕获值 (design D3 纪律)
        let theme_danger = cx.theme().danger;
        // 历史列表: most-recent-first 已由 `History::push` 排好 (构造时读入)。
        // 空历史 → 容器无子行 (spec: empty history list); 打开/创建入口
        // 始终在场 (spec: Launcher offers open and create entries)。
        let entries: Vec<AnyElement> = self
            .history_entries()
            .iter()
            .enumerate()
            .map(|(ix, path)| {
                let label = path.to_string_lossy().into_owned();
                let click_path = path.clone();
                div()
                    .id(format!("launcher-entry-{ix}"))
                    .test_support()
                    .cursor_pointer()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .text_sm()
                    .on_click(cx.listener(move |this, _event, window, cx| {
                        this.choose(click_path.clone(), window.window_handle(), cx);
                    }))
                    .child(label.clone())
                    // texts() 只收录 test_text (pane/tree 同款)
                    .test_text(label)
                    .into_any_element()
            })
            .collect();

        div()
            .id("launcher-root")
            .test_support() // 观测锚点 (无 test-support 特性时为 no-op)
            .size_full()
            .flex()
            .flex_col()
            .p_4()
            .gap_2()
            .child(
                div()
                    .id("launcher-title")
                    .test_support()
                    .text_xl()
                    .child("balabala")
                    .test_text("balabala"),
            )
            // 就地错误 (design D4: 报错留在 Launcher, 不退出)
            .children(self.error.clone().map(|msg| {
                div()
                    .id("launcher-error")
                    .test_support()
                    .text_sm()
                    .text_color(theme_danger)
                    .child(msg.clone())
                    .test_text(msg)
            }))
            .child(
                div()
                    .id("launcher-history")
                    .test_support()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(entries),
            )
            .child(
                div()
                    .id("launcher-open")
                    .test_support()
                    .cursor_pointer()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .border_1()
                    .text_sm()
                    .child("open workspace…")
                    .test_text("open workspace…")
                    .on_click(cx.listener(|this, _event, window, cx| {
                        this.pick_and_choose(window.window_handle(), cx);
                    })),
            )
            .child(
                div()
                    .id("launcher-create")
                    .test_support()
                    .cursor_pointer()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .border_1()
                    .text_sm()
                    .child("new workspace…")
                    .test_text("new workspace…")
                    .on_click(cx.listener(|this, _event, window, cx| {
                        this.pick_and_choose(window.window_handle(), cx);
                    })),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    //! 无头验证 (launcher-startup tasks 1.x / 2.x)。**显式导入, 不用 glob** —
    //! `gpui_kit::*` 会把 `#[test]` 宏遮蔽掉 (app.rs 测试模块同款教训)。

    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AppContext, Entity, TestAppContext, WindowBounds, WindowOptions, px, size};

    use super::{LauncherView, open_with_history};
    use crate::project_model;
    use crate::workspace::history::History;

    /// 进程内序号 — 保证并发测试各自拿到不同历史文件。
    ///
    /// 只用纳秒时间戳会在并行跑时碰撞 (实测: 空历史测试读到别的测试
    /// 种的条目), 序号 + pid 才是确定性的唯一。
    static SEQ: AtomicU64 = AtomicU64::new(0);

    /// hermetic 空历史路径 (进程 + 序号隔离)。
    fn fresh_history_path() -> PathBuf {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("launcher-hist-{}-{n}.json", std::process::id()));
        let _ = std::fs::remove_file(&p);
        p
    }

    /// hermetic 幽灵目录 (确保不存在)。
    fn ghost_dir(tag: &str) -> PathBuf {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("launcher-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        p
    }

    /// 测试用的空 legacy 路径 (确定性不存在 → 播种 no-op, 不碰仓库根)。
    fn empty_legacy_path() -> PathBuf {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("launcher-legacy-none-{}-{n}.json", std::process::id()));
        let _ = std::fs::remove_file(&p);
        p
    }

    /// 测试统一入口: 初始化全局 (Theme 等 — `cx.theme()` 需要) + 开 Launcher。
    ///
    /// 生产路径由 `main.rs` 的 `gpui_kit::init` 完成; 测试环境每例独立
    /// `TestAppContext`, 必须自己 init (app.rs 各测试同款)。
    fn start_launcher(cx: &mut TestAppContext, history: PathBuf) -> gpui_kit::AnyWindowHandle {
        cx.update(gpui_kit::init);
        let legacy = empty_legacy_path();
        cx.update(|cx| open_with_history(cx, history, legacy))
    }

    /// 1.1: 启动只出 Launcher — 唯一窗口, 主窗未构造, 无 workspace。
    /// (spec: Startup opens the Launcher before any workspace)
    #[gpui_kit::test]
    fn startup_opens_only_the_launcher(cx: &mut TestAppContext) {
        let history = fresh_history_path();
        let handle = start_launcher(cx, history);
        cx.run_until_parked();

        let windows = cx.windows();
        assert_eq!(windows.len(), 1, "启动阶段唯一窗口应为 Launcher");
        assert_eq!(
            windows[0].window_id(),
            handle.window_id(),
            "该唯一窗口就是 Launcher 自己 — 主窗 (会是第二个窗口) 尚未构造"
        );
        // 注: `root_entity_type_name()` 对 gpui-kit 窗口恒为
        // `gpui_base::root::Root` (Root 非泛型, 持 AnyView), 区分不了窗口
        // 类型 — 所以用句柄身份而非根实体名来断言 "主窗未构造"。
    }

    /// 1.1: 构造即读历史 (fs, 无服务) — 指定文件里的条目进入列表。
    #[gpui_kit::test]
    fn launcher_reads_history_on_construction(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let history = fresh_history_path();
        let mut seeded = History::default();
        seeded.recent = vec![PathBuf::from("/ws/from-file")];
        seeded.save_at(&history).expect("seed history");

        let (_handle, entity) = cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(480.), px(560.)), cx)),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, {
                let history = history.clone();
                move |_window, cx| cx.new(|cx| LauncherView::new(history, cx))
            })
            .expect("open test window")
        });
        cx.run_until_parked();

        let entries = cx.update(|cx| entity.read(cx).history_entries().to_vec());
        assert_eq!(
            entries,
            vec![PathBuf::from("/ws/from-file")],
            "构造时读入注入文件的条目"
        );
    }

    /// 1.3: 关闭 Launcher 即中断启动 — 窗口清零, 无 workspace。
    /// (`QuitMode::LastWindowClosed` 下它就是最后一个窗口 → 进程退出;
    /// 无头环境不断言进程退出, 断言其前提: 关后无窗、无 workspace。)
    /// (spec: Closing the Launcher interrupts startup)
    #[gpui_kit::test]
    fn closing_the_launcher_interrupts_startup(cx: &mut TestAppContext) {
        let history = fresh_history_path();
        let handle = start_launcher(cx, history);
        cx.run_until_parked();
        assert_eq!(cx.windows().len(), 1, "关闭前 Launcher 在场");

        // 关窗 (正式路径 = 用户点窗口关闭按钮 → 平台 on_close → 同款置位)
        cx.update(|cx| {
            handle
                .update(cx, |_, window, _| window.remove_window())
                .expect("update launcher window");
        });
        cx.run_until_parked();

        assert!(
            cx.windows().is_empty(),
            "关闭后不得残留任何窗口, 实际: {:?}",
            cx.windows()
        );
        // 主窗从未构造 → 无 TerminalApp → 无 workspace 可言
    }

    /// 起一个指定历史内容的 Launcher, 返回 (窗口句柄, 实体)。
    fn launcher_entity(
        cx: &mut TestAppContext,
        entries: Vec<PathBuf>,
    ) -> (gpui_kit::AnyWindowHandle, Entity<LauncherView>) {
        cx.update(gpui_kit::init);
        let history = fresh_history_path();
        let mut seeded = History::default();
        // 直接写 recent — 绕开 push 的排序语义 (这里只喂渲染)
        seeded.recent = entries;
        seeded.save_at(&history).expect("seed history");

        let (handle, entity) = cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(480.), px(560.)), cx)),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, {
                let history = history.clone();
                move |_window, cx| cx.new(|cx| LauncherView::new(history, cx))
            })
            .expect("open test window")
        });
        cx.run_until_parked();
        (handle, entity)
    }

    /// 起一个指定历史内容的 Launcher, 返回 (窗口句柄, 文本列表)。
    fn launcher_with(
        cx: &mut TestAppContext,
        entries: Vec<PathBuf>,
    ) -> (gpui_kit::AnyWindowHandle, Vec<String>) {
        let (handle, _entity) = launcher_entity(cx, entries);
        let texts = render_texts(handle, cx);
        (handle, texts)
    }

    /// 无头 render 文本收集 (app.rs `render_texts` 同款)。
    fn render_texts(handle: gpui_kit::AnyWindowHandle, cx: &mut TestAppContext) -> Vec<String> {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            gpui_kit::base::test_support::texts(window)
                .iter()
                .map(|t| t.text().to_string())
                .collect()
        })
        .expect("update window")
    }

    /// 1.2: 历史条目按 most-recent-first 渲染。
    #[gpui_kit::test]
    fn history_entries_render_most_recent_first(cx: &mut TestAppContext) {
        let a = PathBuf::from("/ws/alpha");
        let b = PathBuf::from("/ws/beta");
        let (_handle, texts) = launcher_with(cx, vec![a, b]);

        let joined = texts.join("\n");
        let ix_a = joined.find("/ws/alpha").expect("alpha 应渲染");
        let ix_b = joined.find("/ws/beta").expect("beta 应渲染");
        assert!(ix_a < ix_b, "most-recent-first: alpha 应在 beta 之前");
    }

    /// 1.2: 空历史 → 列表无条目, 但打开/创建入口仍在场。
    #[gpui_kit::test]
    fn empty_history_still_offers_open_and_create(cx: &mut TestAppContext) {
        let (_handle, texts) = launcher_with(cx, Vec::new());

        assert!(
            !texts.iter().any(|t| t.starts_with("/")),
            "空历史不渲染任何条目行, 实际: {texts:?}"
        );
        assert!(
            texts.iter().any(|t| t == "open workspace…"),
            "打开入口应始终在场, 实际: {texts:?}"
        );
        assert!(
            texts.iter().any(|t| t == "new workspace…"),
            "创建入口应始终在场, 实际: {texts:?}"
        );
    }

    /// 1.2: 有历史时两个入口同样在场 (入口不随列表内容消失)。
    #[gpui_kit::test]
    fn entries_and_actions_coexist(cx: &mut TestAppContext) {
        let (_handle, texts) = launcher_with(cx, vec![PathBuf::from("/ws/gamma")]);
        assert!(texts.iter().any(|t| t == "/ws/gamma"), "条目渲染");
        assert!(texts.iter().any(|t| t == "open workspace…"), "打开入口");
        assert!(texts.iter().any(|t| t == "new workspace…"), "创建入口");
    }

    /// 2.1: 点死条目 → 就地报错、留在 Launcher、进程不退、无 workspace。
    /// (spec: Dead history entry stays in the Launcher)
    #[gpui_kit::test]
    fn dead_history_entry_stays_in_launcher(cx: &mut TestAppContext) {
        let ghost = ghost_dir("dead");
        let (handle, _texts) = launcher_with(cx, vec![ghost.clone()]);

        cx.update_window(handle, |_, window, cx| {
            window.click("launcher-entry-0", cx);
        })
        .expect("click launcher entry");
        cx.run_until_parked();

        let texts = render_texts(handle, cx);
        assert_eq!(
            cx.windows().len(),
            1,
            "Launcher 仍开着, 实际窗口: {:?}",
            cx.windows()
        );
        let joined = texts.join("\n");
        assert!(
            joined.contains("does not exist"),
            "就地报错点名目录不存在, 实际: {joined:?}"
        );
    }

    /// 2.1: 点 marker 损坏的条目 → 就地报错、留在 Launcher、不退出。
    /// (spec: Unparseable marker stays in the Launcher)
    #[gpui_kit::test]
    fn corrupt_marker_stays_in_launcher(cx: &mut TestAppContext) {
        let ws = ghost_dir("corrupt");
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::write(
            ws.join(project_model::files::MARKER_FILE),
            "{ not json",
        )
        .unwrap();

        let (handle, _texts) = launcher_with(cx, vec![ws.clone()]);
        cx.update_window(handle, |_, window, cx| {
            window.click("launcher-entry-0", cx);
        })
        .expect("click launcher entry");
        cx.run_until_parked();

        let texts = render_texts(handle, cx);
        assert_eq!(
            cx.windows().len(),
            1,
            "Launcher 仍开着, 实际窗口: {:?}",
            cx.windows()
        );
        let joined = texts.join("\n");
        assert!(
            joined.contains("parse"),
            "错误点名解析失败, 实际: {joined:?}"
        );
        let _ = std::fs::remove_dir_all(&ws);
    }

    /// 2.2: 拒绝创建 → 无文件、无 workspace、Launcher 仍开、无报错。
    /// (spec: declining leaves the Launcher open with no workspace)
    #[gpui_kit::test]
    fn declining_creation_leaves_launcher_untouched(cx: &mut TestAppContext) {
        let target = ghost_dir("decline");
        std::fs::create_dir_all(&target).unwrap();
        let (handle, entity) = launcher_entity(cx, Vec::new());

        cx.update(|cx| {
            entity.update(cx, |view, cx| {
                view.on_create_decision(false, target.clone(), &handle, cx);
            });
        });
        cx.run_until_parked();

        assert!(
            !target.join(project_model::files::MARKER_FILE).exists(),
            "拒绝后不得写 marker"
        );
        assert!(
            !target.join(project_model::files::SOLUTIONS_FILE).exists(),
            "拒绝后不得写 solutions"
        );
        assert_eq!(
            cx.windows().len(),
            1,
            "拒绝后 Launcher 仍开, 实际: {:?}",
            cx.windows()
        );
        assert_eq!(
            cx.update(|cx| entity.read(cx).error().map(str::to_string)),
            None,
            "拒绝不是错误"
        );
        let _ = std::fs::remove_dir_all(&target);
    }

    /// 2.2: 确认创建 → 两文件落盘 (spec: Confirmed creation opens the workspace)
    #[gpui_kit::test]
    fn confirming_creation_writes_marker_and_solutions(cx: &mut TestAppContext) {
        let target = ghost_dir("confirm");
        std::fs::create_dir_all(&target).unwrap();
        let (handle, entity) = launcher_entity(cx, Vec::new());

        cx.update(|cx| {
            entity.update(cx, |view, cx| {
                view.on_create_decision(true, target.clone(), &handle, cx);
            });
        });
        cx.run_until_parked();

        assert!(
            target.join(project_model::files::MARKER_FILE).exists(),
            "确认后 marker 落盘"
        );
        assert!(
            target.join(project_model::files::SOLUTIONS_FILE).exists(),
            "确认后 solutions 落盘"
        );
        assert_eq!(
            cx.update(|cx| entity.read(cx).error().map(str::to_string)),
            None,
            "创建成功不报错"
        );
        // 交棒: 主窗已开, Launcher 已关 (design D1 顺序) —
        // 用句柄身份断言: 剩下的那个窗口不是 Launcher, 即主窗。
        let windows = cx.windows();
        assert_eq!(windows.len(), 1, "交棒后只剩一个窗口, 实际: {windows:?}");
        assert_ne!(
            windows[0].window_id(),
            handle.window_id(),
            "剩下的窗口不是 Launcher — 它已关, 主窗已开"
        );
        let _ = std::fs::remove_dir_all(&target);
    }

    /// 2.3: 合法路径 → 开主窗 + 同批关 Launcher (先开后关, 无零窗口瞬退)。
    /// (spec: Choosing a history entry opens that workspace)
    #[gpui_kit::test]
    fn valid_entry_hands_off_to_main_window(cx: &mut TestAppContext) {
        // 造一个合法工作区 (marker + 空 solutions)
        let ws = ghost_dir("valid");
        std::fs::create_dir_all(&ws).unwrap();
        project_model::create_workspace_files(&ws).unwrap();

        let (handle, entity) = launcher_entity(cx, vec![ws.clone()]);
        assert_eq!(cx.windows().len(), 1, "交棒前只有 Launcher");

        cx.update_window(handle, |_, window, cx| {
            window.click("launcher-entry-0", cx);
        })
        .expect("click launcher entry");
        cx.run_until_parked();

        let windows = cx.windows();
        assert_eq!(
            windows.len(),
            1,
            "交棒后只剩一个窗口 (无零窗口瞬退), 实际: {windows:?}"
        );
        assert_ne!(
            windows[0].window_id(),
            handle.window_id(),
            "Launcher 已关, 留下的是主窗"
        );
        assert!(
            cx.update(|cx| entity.read(cx).error().map(str::to_string)).is_none(),
            "合法路径不就地报错"
        );
        let _ = std::fs::remove_dir_all(&ws);
    }

    /// 5.1: Launcher 期菜单栏至少一个菜单, 且不含依赖主窗的 Debug 子菜单。
    /// (spec: Application menu bar is always present — at startup in all
    ///  build profiles; design D5 两步装的第一步)
    #[gpui_kit::test]
    fn launcher_installs_at_least_one_menu(cx: &mut TestAppContext) {
        let history = fresh_history_path();
        let _handle = start_launcher(cx, history);
        cx.run_until_parked();

        let menus = cx
            .update(|cx| cx.get_menus())
            .expect("Launcher 期菜单已安装");
        let names: Vec<&str> = menus.iter().map(|m| m.name.as_str()).collect();
        assert!(!names.is_empty(), "至少一个菜单, 实际: {names:?}");
        assert!(
            names.contains(&"Edit"),
            "Launcher 期装 Edit, 实际: {names:?}"
        );
        assert!(
            !names.contains(&"Debug"),
            "Debug 子菜单依赖 TerminalApp, Launcher 期不该在场, 实际: {names:?}"
        );
    }

    /// 2.4: legacy 播种在呈现历史之前完成, 且 legacy 文件不被改写。
    /// (spec: Seeding SHALL complete before the Launcher presents history;
    ///  the application SHALL NOT rewrite the legacy file)
    #[gpui_kit::test]
    fn legacy_seeding_completes_before_history_is_presented(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let history = fresh_history_path();
        let legacy = empty_legacy_path();
        std::fs::write(
            &legacy,
            r#"{"workspaces":["/legacy/one","/legacy/two"]}"#,
        )
        .expect("write legacy");
        let legacy_before = std::fs::read_to_string(&legacy).expect("read legacy");

        let handle = cx.update(|cx| open_with_history(cx, history.clone(), legacy.clone()));
        cx.run_until_parked();
        let texts = render_texts(handle, cx);
        let joined = texts.join("\n");

        assert!(
            joined.contains("/legacy/one") && joined.contains("/legacy/two"),
            "播种后的条目出现在 Launcher 列表, 实际: {joined:?}"
        );
        assert_eq!(
            std::fs::read_to_string(&legacy).expect("re-read legacy"),
            legacy_before,
            "播种不得改写 legacy 文件"
        );
        // 播种已落盘 (legacy_seeded 置位) — 再开一次不再重复播种
        let reopened = cx.update(|cx| open_with_history(cx, history, legacy.clone()));
        cx.run_until_parked();
        let texts2 = render_texts(reopened, cx);
        let count = texts2.iter().filter(|t| t.as_str() == "/legacy/one").count();
        assert_eq!(count, 1, "二次打开不重复播种, 实际: {texts2:?}");
    }
}
