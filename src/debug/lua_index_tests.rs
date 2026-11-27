//! lua-index section 无头契约 (task 3.3): 真实装配链开签 → dedup →
//! 注入前空命中 → 注入 fixture 后命中/诊断行渲染。
//!
//! 与 view_tests 的分工: crate 侧测**视图自身** (输入/按钮缝), 本文件测
//! **主 crate 集成链** (register_builtin → 开签 → 面板 → 渲染服务状态)。
//! 注入直接走服务 API (`set_roots`) — 输入驱动路径已在 view_tests 覆盖。
//!
//! 显式导入, 不用 glob — 防 `gpui_kit::*` 把 `#[test]` 解析到
//! `gpui_kit::test` (panel_tests 同款教训)。helper 为 term_rate_tests
//! 同款本地副本 (codebase 惯例: 样板本地复制不抽公共)。

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use gpui_kit::component::dock::PanelHandle;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, Entity, Point, TestAppContext, WindowBounds,
    WindowOptions, px, size,
};

use crate::app::TerminalApp;
use crate::debug::panel::DebugSectionPanel;

/// 进程级服务单例的串行锁 (本测试是主 crate 内唯一消费者, 预留同款纪律)。
static TEST_LOCK: Mutex<()> = Mutex::new(());

// ---------- helper (term_rate_tests 同款副本) ----------

fn open_app(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<TerminalApp>) {
    cx.update(gpui_kit::init);
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
            |window, cx| cx.new(|cx| TerminalApp::new(window, cx, None).expect("app 启动")),
        )
        .expect("open test window")
    });
    app.update(cx, |app, _| {
        app.workspace = None;
    });
    (handle, app)
}

/// 走平行入口开一个 section (真实调用链: update_window → app.update)。
fn open_section_in(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    app: &Entity<TerminalApp>,
    id: &str,
) {
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |app, cx| app.open_debug_section(id, window, cx));
    })
    .expect("update window");
    cx.run_until_parked();
}

/// section 面板实体 (dedup 表 → dock → 两跳 downcast)。
fn section_entity(
    cx: &mut TestAppContext,
    app: &Entity<TerminalApp>,
    id: &str,
) -> Entity<DebugSectionPanel> {
    cx.update(|cx| {
        let app = app.read(cx);
        let panel_id = *app.debug_tabs.get(id).expect("dedup 表登记 section");
        let area = app.dock_area.read(cx);
        let handle = area.panel(panel_id).expect("section 在 dock 中");
        let wrapped = PanelHandle::of(handle).expect("panel_handle 包装");
        wrapped
            .panel()
            .as_any()
            .downcast_ref::<Entity<DebugSectionPanel>>()
            .expect("inner 是 Entity<DebugSectionPanel>")
            .clone()
    })
}

/// 渲染一帧并取全部文本 (term_rate render_texts 同款)。
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

// ---------- 3.3 验收 ----------

/// 写 fixture 目录 (pid 隔离) — crate 侧 view_tests 同款本地副本。
fn write_fixture(tag: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lua-sec-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (rel, content) in files {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().expect("fixture parent")).expect("mkdir fixture");
        std::fs::write(&path, content).expect("write fixture");
    }
    dir
}

/// 轮询服务到分析终点 (服务是真后台线程, 与 gpui executor 无关)。
fn wait_idle() -> lua_language_service::IndexStatus {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let st = lua_language_service::index_status();
        match st.phase {
            lua_language_service::IndexPhase::Idle | lua_language_service::IndexPhase::Failed => {
                return st;
            }
            lua_language_service::IndexPhase::Empty | lua_language_service::IndexPhase::InProgress => {
                if Instant::now() >= deadline {
                    panic!("等待 lua 索引落定超时: {st:?}");
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    }
}

/// 单测试多阶段 (term_rate 同款): 每个测试 = 一个完整 app 启动,
/// 合并控制负载。注入前空态 / 注入后命中+诊断 / dedup 全在一个测试里。
#[gpui_kit::test]
fn lua_index_section_open_dedup_and_render(cx: &mut TestAppContext) {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    // 进程级单例基线复位 (防同二进制内其它测试残留)。
    lua_language_service::set_roots(Vec::new());

    let dir = write_fixture(
        "sec",
        &[
            ("src/net/client.lua", "function on_ready()\nend\n"),
            ("broken.lua", "local x = (\n"),
        ],
    );

    // open_app = 真实装配链: TerminalApp::new → register_builtin (lua-index
    // 登记) → menu::install。
    let (handle, app) = open_app(cx);
    assert!(
        crate::debug::section_by_id("lua-index").is_some(),
        "装配链须登记 lua-index (spec: Debug menu lists a lua-index section)"
    );

    // ---- 开签 + dedup (spec: dedup identity is the section id) ----
    open_section_in(cx, handle, &app, "lua-index");
    let first = section_entity(cx, &app, "lua-index");
    open_section_in(cx, handle, &app, "lua-index");
    let second = section_entity(cx, &app, "lua-index");
    assert_eq!(first, second, "同 id 二次开签须 dedup 到同一面板");

    // ---- 注入前: 空命中 (spec scenario: Search before injection shows nothing) ----
    let texts = render_texts(handle, cx);
    for expect in ["hits=0", "phase=Empty", "roots=(none)", "diagnostics=0"] {
        assert!(
            texts.iter().any(|t| t.contains(expect)),
            "注入前须含 {expect:?}\n实际 texts={texts:?}"
        );
    }

    // ---- 注入 fixture → 后台索引 → 渲染命中/诊断 ----
    lua_language_service::set_roots(vec![dir.clone()]);
    let st = wait_idle();
    assert_eq!(
        st.phase,
        lua_language_service::IndexPhase::Idle,
        "注入后须落定: {st:?}"
    );
    assert_eq!(st.file_count, 2, "两文件计入");

    // 1s tick → notify → 重绘 (term_rate 同款时钟推进)。
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();

    let texts = render_texts(handle, cx);
    for expect in ["files=2", "phase=Idle", "hits=", "diagnostics="] {
        assert!(
            texts.iter().any(|t| t.contains(expect)),
            "注入后须含 {expect:?}\n实际 texts={texts:?}"
        );
    }
    assert!(
        texts.iter()
            .any(|t| t.starts_with("on_ready fn ") && t.contains("client.lua:1:10")),
        "命中行 `on_ready fn <file>:1:10` 须渲染\n实际 texts={texts:?}"
    );
    assert!(
        texts.iter()
            .any(|t| t.starts_with("error ") && t.contains("broken.lua")),
        "诊断行 `error <file>…` 须渲染\n实际 texts={texts:?}"
    );

    // 收尾复位 (不污染同进程其它测试)。
    lua_language_service::set_roots(Vec::new());
    let _ = std::fs::remove_dir_all(&dir);
}
