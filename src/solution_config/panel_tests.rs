//! solution-config-tab 无头契约（tasks 2.3 / 3.2 / 3.3）。
//!
//! 场景（specs/center-tabs delta）：入口开签在 console 后、单实例再开选中
//! 既有签、关后重开 fresh；（specs/project-model delta）：预检三行与提交
//! 门控、提交成功自关 + 追加 item、失败行内显示且签保留、未提交关闭零
//! 写入、workspace 外 root 接受。
//!
//! rfd（browse 按钮）不触碰 — headed 冒烟覆盖（task 4.2）。
//! 显式导入, 不用 glob — 防 `gpui_kit::*` 把属性宏内层 `#[test]`
//! 解析到 `gpui_kit::test` → 递归爆（app.rs 同款教训）。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use gpui_kit::base::dock::PanelId;
use gpui_kit::component::dock::DockPlacement;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, Entity, Point, TestAppContext, WindowBounds,
    WindowOptions, px, size,
};

use crate::app::TerminalApp;
use crate::project_model;
use crate::solution_config::SOLUTION_CONFIG_TAB_KEY;

// ---------- 样板（debug/panel_tests 同款本地副本 + hermetic 历史） ----------

fn fresh_history_path() -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "sct-hist-{}-{}.json",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
    ));
    let _ = fs::remove_file(&p);
    p
}

fn open_app(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<TerminalApp>) {
    cx.update(gpui_kit::init);
    let history = fresh_history_path();
    let (handle, app) = cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(1000.), px(600.)),
                })),
                ..Default::default()
            },
            cx,
            |window, cx| {
                cx.new(|cx| {
                    TerminalApp::new_inner(window, cx, history.clone(), None).expect("app 启动")
                })
            },
        )
        .expect("open test window")
    });
    (handle, app)
}

/// 中心 tab 组（panels 预序, active_ix）— 签序断言观测口。
fn center_tabs(
    cx: &mut TestAppContext,
    app: &Entity<TerminalApp>,
) -> (Vec<PanelId>, usize) {
    cx.update(|cx| {
        let app = app.read(cx);
        let area = app.dock_area.read(cx);
        let tree = area.layout(DockPlacement::Center).expect("center 槽在场");
        let mut found = (Vec::new(), 0usize);
        tree.root().walk(&mut |node| {
            if let gpui_kit::base::dock::layout::PaneRef::Tabs { panels, active_ix } = node.kind()
                && found.0.is_empty()
            {
                found = (panels.to_vec(), active_ix);
            }
        });
        found
    })
}

fn console_id(cx: &mut TestAppContext, app: &Entity<TerminalApp>) -> PanelId {
    cx.update(|cx| PanelId::from(app.read(cx).terminal.entity_id()))
}

fn render_texts(handle: AnyWindowHandle, cx: &mut TestAppContext) -> Vec<String> {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        gpui_kit::base::test_support::texts(window)
            .iter()
            .map(|t| t.text().to_string())
            .collect()
    })
    .expect("update window")
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "sct-{tag}-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// 泵（poll_sessions 驱动 mailbox 消费; run_until_parked 只冲测试执行器）。
fn pump(cx: &mut TestAppContext, app: &Entity<TerminalApp>) {
    cx.run_until_parked();
    app.update(cx, |app, cx| {
        app.poll_sessions(cx);
    });
    cx.run_until_parked();
}

fn wait_for(
    cx: &mut TestAppContext,
    app: &Entity<TerminalApp>,
    mut pred: impl FnMut(&TerminalApp) -> bool,
    what: &str,
) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        pump(cx, app);
        if cx.update(|cx| pred(app.read(cx))) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "未落定: {what}"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// 让服务落盘必败 (design D5): 把 solutions 文件换成同名目录 →
/// `fs::write` 必 EISDIR。落盘目标由服务绑定, 不能再靠改 view 侧的
/// `workspace.path` 来影响写入位置。
fn break_persist(ws: &std::path::Path) {
    let p = project_model::files::solutions_path(ws);
    let _ = fs::remove_file(&p);
    fs::create_dir_all(&p).unwrap();
}

/// 起一个空工作区并打开（hermetic）。
fn seed_and_open(cx: &mut TestAppContext, app: &Entity<TerminalApp>, tag: &str) -> PathBuf {
    let ws = temp_dir(tag);
    project_model::create_workspace_files(&ws).unwrap();
    app.update(cx, |app, cx| app.begin_open(ws.clone(), cx));
    wait_for(cx, app, |a| a.workspace.is_some(), "打开空工作区");
    ws
}

/// 走真实入口开签。
fn open_entry_in(cx: &mut TestAppContext, handle: AnyWindowHandle, app: &Entity<TerminalApp>) {
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |app, cx| app.open_solution_config_tab(window, cx));
    })
    .expect("update window");
    cx.run_until_parked();
}

/// 灌 path（测试缝 set_path; 需 window — update_window 内直调）。
fn set_path_in(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    app: &Entity<TerminalApp>,
    path: &Path,
) {
    cx.update_window(handle, |_, window, cx| {
        let panel = app
            .read(cx)
            .sol_config_panel
            .clone()
            .expect("表单签在场");
        panel.update(cx, |panel, cx| panel.set_path(&path.to_string_lossy(), window, cx));
    })
    .expect("update window");
    cx.run_until_parked();
}

/// 模拟用户关签：**不经 TerminalApp 租约**直接 remove（on_removed 重入
/// TerminalApp — app.update 内嵌套即 double-lease panic, 见 close 文档）。
fn close_tab_user_style(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    app: &Entity<TerminalApp>,
) {
    cx.update_window(handle, |_, window, cx| {
        let (dock, panel) = {
            let a = app.read(cx);
            (
                a.dock_area.clone(),
                a.sol_config_panel.clone().expect("表单签在场"),
            )
        };
        dock.update(cx, |area, cx| area.remove_panel(panel, window, cx));
    })
    .expect("update window");
    cx.run_until_parked();
}

fn solutions_json(ws: &Path) -> String {
    fs::read_to_string(project_model::files::solutions_path(ws)).unwrap_or_default()
}

// ---------- 2.3: 入口开签 / 单实例 / 关后重开 ----------

#[gpui_kit::test]
fn entry_opens_tab_after_console_and_dedups(cx: &mut TestAppContext) {
    let (handle, app) = open_app(cx);
    cx.run_until_parked();

    open_entry_in(cx, handle, &app);
    let (panels, active) = center_tabs(cx, &app);
    assert_eq!(panels.len(), 2, "console + solution-config 各一签");
    assert_eq!(panels[0], console_id(cx, &app), "console 恒 index 0");
    assert_eq!(panels[1], panels[active], "新签即 active");
    assert!(
        cx.update(|cx| app.read(cx).open_tabs.contains_key(SOLUTION_CONFIG_TAB_KEY)),
        "dedup key 已登记"
    );
    let texts = render_texts(handle, cx);
    assert!(
        texts.iter().any(|t| t.contains("project path")),
        "表单骨架应渲染: {texts:?}"
    );
    assert!(
        texts.iter().any(|t| t.contains("path not empty")),
        "预检行应渲染: {texts:?}"
    );

    // 切回 console 后再点入口 → 选中既有签, 不新建 (单实例)
    let config = panels[1];
    let console = console_id(cx, &app);
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |app, cx| {
            app.dock_area
                .update(cx, |area, cx| area.select_panel(console, window, cx));
        });
    })
    .expect("update window");
    open_entry_in(cx, handle, &app);
    let (panels2, active2) = center_tabs(cx, &app);
    assert_eq!(panels2.len(), 2, "再开不新建 (spec: Re-entry selects the single tab)");
    assert_eq!(panels2[active2], config, "选中既有签");
}

#[gpui_kit::test]
fn close_tab_reopens_fresh(cx: &mut TestAppContext) {
    let (handle, app) = open_app(cx);
    cx.run_until_parked();

    open_entry_in(cx, handle, &app);
    let (panels, _) = center_tabs(cx, &app);
    let first = panels[1];

    close_tab_user_style(cx, handle, &app);
    assert!(
        !cx.update(|cx| app.read(cx).open_tabs.contains_key(SOLUTION_CONFIG_TAB_KEY)),
        "关签摘 dedup (spec: Closing the form reopens fresh)"
    );
    assert_eq!(center_tabs(cx, &app).0.len(), 1, "回 console 单签");

    open_entry_in(cx, handle, &app);
    let (panels2, active2) = center_tabs(cx, &app);
    assert_eq!(panels2.len(), 2, "重开成功");
    assert_ne!(panels2[active2], first, "fresh = 新 PanelId");
}

// ---------- 3.3: 预检三行与门控 ----------

#[gpui_kit::test]
fn precheck_rows_gate_submit(cx: &mut TestAppContext) {
    let (handle, app) = open_app(cx);
    cx.run_until_parked();
    seed_and_open(cx, &app, "precheck");

    // 空输入: not_empty fail → 门控
    open_entry_in(cx, handle, &app);
    assert!(
        cx.update(|cx| {
            let a = app.read(cx);
            let p = a.sol_config_panel.as_ref().expect("签在场");
            !p.read(cx).can_submit(cx)
        }),
        "空输入不可提交"
    );
    let texts = render_texts(handle, cx);
    assert!(
        texts.iter().any(|t| t.contains("path not empty: fail")),
        "空输入失败行可见: {texts:?}"
    );

    // 非法路径 (不存在): existing_dir fail
    let ghost = std::env::temp_dir().join(format!("sct-ghost-{}", std::process::id()));
    set_path_in(cx, handle, &app, &ghost);
    assert!(
        cx.update(|cx| {
            let a = app.read(cx);
            !a.sol_config_panel.as_ref().unwrap().read(cx).can_submit(cx)
        }),
        "不存在路径不可提交 (防下次装载 MissingPath 阻断)"
    );
    let texts = render_texts(handle, cx);
    assert!(
        texts.iter().any(|t| t.contains("existing directory: fail")),
        "合法性失败行可见: {texts:?}"
    );

    // 合法且在 workspace 外 (决策 2026-11-23: root 只需合法) → 全过
    let outside = temp_dir("outside");
    set_path_in(cx, handle, &app, &outside);
    assert!(
        cx.update(|cx| {
            let a = app.read(cx);
            let p = a.sol_config_panel.as_ref().unwrap();
            p.read(cx).can_submit(cx) && p.read(cx).prechecks(cx).all_pass()
        }),
        "workspace 外的合法 root 全过 (spec: Root outside the workspace is accepted)"
    );
    let texts = render_texts(handle, cx);
    assert!(texts.iter().any(|t| t.contains("existing directory: ok")), "{texts:?}");
    assert!(texts.iter().any(|t| t.contains("unique solution: ok")), "{texts:?}");
}

// ---------- 3.3: 提交成功 → 自关 + 追加 item（外部 root 接受） ----------

#[gpui_kit::test]
fn submit_success_closes_tab_and_appends(cx: &mut TestAppContext) {
    let (handle, app) = open_app(cx);
    cx.run_until_parked();
    let ws = seed_and_open(cx, &app, "submit");
    let outside = temp_dir("submit-root"); // workspace 外

    open_entry_in(cx, handle, &app);
    set_path_in(cx, handle, &app, &outside);

    // 提交（panel 缝 = click 监听器主体, design D3）
    let panel = cx.update(|cx| app.read(cx).sol_config_panel.clone().expect("签在场"));
    panel.update(cx, |panel, cx| panel.submit(cx));

    wait_for(
        cx,
        &app,
        |a| a.service_layer.tree_view().is_some_and(|t| t.tree().projects.len() == 1),
        "create 落定",
    );
    // 成功自关 (spec: Successful creation closes the tab)
    wait_for(
        cx,
        &app,
        |a| !a.open_tabs.contains_key(SOLUTION_CONFIG_TAB_KEY),
        "成功自关签",
    );
    assert!(!cx.update(|cx| app.read(cx).sol_config_inflight), "inflight 已清");
    assert_eq!(center_tabs(cx, &app).0.len(), 1, "签已移除, 回 console 单签");
    // 文件落盘 (spec: append + Root outside the workspace is accepted)
    let json = solutions_json(&ws);
    assert!(
        json.contains(&outside.to_string_lossy().to_string()),
        "solutions 文件应含该 root: {json}"
    );
    assert!(cx.update(|cx| app.read(cx).sol_config_error.is_none()), "成功无行内错误");
}

// ---------- 3.3: 重名预检 → 重复提交零变化 ----------

#[gpui_kit::test]
fn duplicate_root_blocks_second_submit(cx: &mut TestAppContext) {
    let (handle, app) = open_app(cx);
    cx.run_until_parked();
    let ws = seed_and_open(cx, &app, "dup");
    let root = temp_dir("dup-root");

    // 先经既有缝建一次 (项目在树)
    app.update(cx, |app, cx| app.request_create_solution(root.clone(), cx));
    wait_for(
        cx,
        &app,
        |a| a.service_layer.tree_view().is_some_and(|t| t.tree().projects.len() == 1),
        "首次 create 落定",
    );

    // 新签同 path → 重名 fail, 提交被门控拦下
    open_entry_in(cx, handle, &app);
    set_path_in(cx, handle, &app, &root);
    assert!(
        cx.update(|cx| {
            let a = app.read(cx);
            !a.sol_config_panel.as_ref().unwrap().read(cx).can_submit(cx)
        }),
        "重名 root 不可提交 (spec: Failed precheck blocks submission)"
    );
    let texts = render_texts(handle, cx);
    assert!(
        texts.iter().any(|t| t.contains("unique solution: fail")),
        "重名失败行可见: {texts:?}"
    );
    let panel = cx.update(|cx| app.read(cx).sol_config_panel.clone().unwrap());
    panel.update(cx, |panel, cx| panel.submit(cx)); // 门控: 无动作
    wait_for(cx, &app, |_| true, "泵一轮");
    cx.update(|cx| {
        let a = app.read(cx);
        let t = a.service_layer.tree_view().expect("镜像在场");
        assert_eq!(t.tree().projects.len(), 1, "重复提交零变化 (spec: Re-adding …)");
    });
    let json = solutions_json(&ws);
    assert_eq!(
        json.matches(&root.to_string_lossy().to_string()).count(),
        1,
        "文件仅一条 item: {json}"
    );
}

// ---------- 3.3: 未提交关闭 → 零写入 ----------

#[gpui_kit::test]
fn close_without_submit_writes_nothing(cx: &mut TestAppContext) {
    let (handle, app) = open_app(cx);
    cx.run_until_parked();
    let ws = seed_and_open(cx, &app, "close-零写");
    let outside = temp_dir("close-root");
    let before = solutions_json(&ws);

    open_entry_in(cx, handle, &app);
    set_path_in(cx, handle, &app, &outside);
    close_tab_user_style(cx, handle, &app);

    assert_eq!(solutions_json(&ws), before, "关签不写文件 (spec: Closing the tab writes nothing)");
    cx.update(|cx| {
        let a = app.read(cx);
        assert!(
            a.service_layer
                .tree_view()
                .is_none_or(|t| t.tree().projects.is_empty()),
            "无项目行出现"
        );
    });
}

// ---------- 3.1/3.2: 写失败 → 行内错误 + 签保留 ----------

#[gpui_kit::test]
fn write_failure_shows_inline_and_tab_stays(cx: &mut TestAppContext) {
    let (handle, app) = open_app(cx);
    cx.run_until_parked();
    let ws = seed_and_open(cx, &app, "fail-行内");
    let outside = temp_dir("fail-root");

    // 服务落盘必败: solutions 文件换成同名目录 (design D5 后落盘目标由
    // 服务绑定, 改 view 侧 `workspace.path` 已不影响写入位置)
    break_persist(&ws);

    open_entry_in(cx, handle, &app);
    set_path_in(cx, handle, &app, &outside);
    let panel = cx.update(|cx| app.read(cx).sol_config_panel.clone().unwrap());
    panel.update(cx, |panel, cx| panel.submit(cx));

    wait_for(cx, &app, |a| a.notice.is_some(), "写失败 notice");
    cx.update(|cx| {
        let a = app.read(cx);
        // 路由按 action_id (design D7) — 落签即证明, 不再匹配文案
        assert!(
            a.sol_config_error.is_some(),
            "行内错误落签 (spec: failure → error shown in the tab): {:?}",
            a.sol_config_error
        );
        assert!(
            a.open_tabs.contains_key(SOLUTION_CONFIG_TAB_KEY),
            "失败时签保留 (spec: the tab stays)"
        );
        assert!(!a.sol_config_inflight, "失败回执清 inflight");
        assert!(
            a.service_layer
                .tree_view()
                .is_some_and(|t| t.tree().projects.len() == 1),
            "内存变更保留 (spec: in-memory state keeps the change)"
        );
    });
    // 可见性按元素 id 断言 — label 只是显示, 文案匹配是 D7 要拆的坏味道
    let visible = cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.try_find("solution-config-error").is_some()
    })
    .expect("update window");
    assert!(visible, "错误行内可见 (按 id 断言)");
}

// ---------- 3.2: 关签后才到的失败回执 → notice 不丢 ----------

#[gpui_kit::test]
fn failure_reply_after_close_still_surfaces_notice(cx: &mut TestAppContext) {
    let (handle, app) = open_app(cx);
    cx.run_until_parked();
    let ws = seed_and_open(cx, &app, "fail-关签竞态");
    let outside = temp_dir("fail-late-root");

    // 落盘必败: solutions 文件换成同名目录 (同 write_failure 同款)
    break_persist(&ws);

    open_entry_in(cx, handle, &app);
    set_path_in(cx, handle, &app, &outside);
    let panel = cx.update(|cx| app.read(cx).sol_config_panel.clone().unwrap());
    panel.update(cx, |panel, cx| panel.submit(cx));
    // 回执到达前关签 (kick 的 8ms 快泵尚未让时 — 关在先, 回执晚到)
    close_tab_user_style(cx, handle, &app);

    wait_for(cx, &app, |a| a.notice.is_some(), "关签后失败回执仍落 notice");
    cx.update(|cx| {
        let a = app.read(cx);
        assert!(
            a.notice.is_some(),
            "关签后 notice 不丢: {:?}",
            a.notice
        );
        // 路由按 action_id (design D7): 签虽已关, 行内错误仍按 id 落定
        assert!(
            a.sol_config_error.is_some(),
            "失败按 action_id 路由到表单签: {:?}",
            a.sol_config_error
        );
        assert!(
            !a.open_tabs.contains_key(SOLUTION_CONFIG_TAB_KEY),
            "签已关"
        );
        assert!(!a.sol_config_inflight, "回执已消费");
        assert!(
            a.service_layer
                .tree_view()
                .is_some_and(|t| t.tree().projects.len() == 1),
            "内存变更保留 (spec)"
        );
    });
}
