//! LuaIndexView 渲染契约 (task 3.1): 注入 fixtures → 状态/命中/诊断行渲染。
//!
//! 视图**自持** (不依赖 TerminalApp) — 测试直接以 view 为窗口根, 比
//! panel_tests 的整机启动轻。状态是进程级单例 → 与 service 测试共用
//! `TEST_LOCK`。断言行文与渲染同源 (status_row/hit_row/diag_row)。
//!
//! 显式导入, 不用 glob — 防 `gpui_kit::*` 把 `#[test]` 解析到
//! `gpui_kit::test` (panel_tests 同款教训)。

use std::path::PathBuf;
use std::sync::MutexGuard;
use std::time::Duration;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, Entity, Point, TestAppContext, WindowBounds,
    WindowOptions, px, size,
};

use super::LuaIndexView;
use crate::service::{TEST_LOCK, index_status, set_roots, wait_settled};
use crate::types::IndexPhase;

fn guard() -> MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

/// 写 fixture 目录 (pid 隔离) — service_tests 同款本地副本。
fn write_fixture(tag: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lua-view-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (rel, content) in files {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().expect("fixture parent")).expect("mkdir fixture");
        std::fs::write(&path, content).expect("write fixture");
    }
    dir
}

/// 以 view 为根开窗 (无需 TerminalApp — 视图自持)。
fn open_view(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<LuaIndexView>) {
    cx.update(gpui_kit::init);
    cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(800.), px(600.)),
                })),
                ..Default::default()
            },
            cx,
            |_window, cx| cx.new(|cx| LuaIndexView::new(cx)),
        )
        .expect("open view window")
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

/// 设输入值 (走 set_value 测试缝, 同 solution_config)。
fn set_input(
    handle: AnyWindowHandle,
    view: &Entity<LuaIndexView>,
    which: InputSlot,
    text: &str,
    cx: &mut TestAppContext,
) {
    let text = text.to_string();
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            let input = match which {
                InputSlot::Root => view.root_input.clone(),
                InputSlot::Query => view.query_input.clone(),
            }
            .expect("首帧已建输入");
            input.update(cx, |state, cx| state.set_value(text.as_str(), window, cx));
        });
    })
    .expect("set input");
    cx.run_until_parked(); // observe → notify → 重绘排队
}

enum InputSlot {
    Root,
    Query,
}

#[gpui_kit::test]
fn renders_status_hits_and_diagnostics(cx: &mut TestAppContext) {
    let _g = guard();
    set_roots(Vec::new()); // 基线复位 (进程级单例)

    let dir = write_fixture(
        "rows",
        &[
            ("src/net/client.lua", "function on_ready()\nend\n"),
            ("broken.lua", "local x = (\n"),
        ],
    );

    let (handle, view) = open_view(cx);

    // ---- 阶段 A: 空态 (spec scenario: Search before injection shows nothing) ----
    let texts = render_texts(handle, cx);
    for expect in [
        "files=0",
        "elapsed=-",
        "phase=Empty",
        "roots=(none)",
        "hits=0",
        "diagnostics=0",
    ] {
        assert!(
            texts.iter().any(|t| t.contains(expect)),
            "空态行须含 {expect:?}\n实际 texts={texts:?}"
        );
    }
    // 注入控制全家桶: 手输 + inject + 系统目录选择框 (browse…) 在场。
    // 对话框本体是正式构建胶水, 无头不触碰 (solution_config 同款纪律) —
    // 只断言控件渲染, 选择→填输入→注入的链路由 inject 缝测试覆盖。
    for control in ["root-label", "inject", "browse…"] {
        assert!(
            texts.iter().any(|t| t == control),
            "注入控件 {control:?} 须渲染\n实际 texts={texts:?}"
        );
    }
    assert!(
        !texts.iter().any(|t| t.contains(" fn ")),
        "空态不得有命中行: {texts:?}"
    );

    // ---- 阶段 B: 经注入控制 (输入 + inject 缝) 注入 ----
    set_input(handle, &view, InputSlot::Root, dir.to_str().expect("utf8"), cx);
    cx.update_window(handle, |_, _window, cx| {
        view.update(cx, |view, cx| view.inject(cx));
    })
    .expect("inject");

    // inject 同步半边: 状态即刻离开 Empty (spec: submission replaces)。
    let st = index_status();
    assert_ne!(
        st.phase,
        IndexPhase::Empty,
        "inject 后状态须立即离开 Empty: {st:?}"
    );

    let st = wait_settled();
    assert_eq!(st.phase, IndexPhase::Idle, "索引落定: {st:?}");
    assert_eq!(st.file_count, 2, "两文件计入");

    // tick → notify → 重绘 (1s 周期, term_rate 同款推进)。
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();

    // ---- 阶段 C: 注入后行文 (spec: file count / elapsed / phase + hits + diagnostics) ----
    let texts = render_texts(handle, cx);
    for expect in [
        "files=2",
        "phase=Idle",
        "roots=", // 注入根在场
        "hits=",
        "diagnostics=",
    ] {
        assert!(
            texts.iter().any(|t| t.contains(expect)),
            "注入后行须含 {expect:?}\n实际 texts={texts:?}"
        );
    }
    // 命中行: 空查询列出全部符号 (注入即见) — `name kind file:line:col`
    assert!(
        texts.iter().any(|t| t.starts_with("on_ready fn ")
            && t.contains("client.lua:1:10")),
        "命中行 `on_ready fn <file>:1:10`\n实际 texts={texts:?}"
    );
    // 诊断行: `severity file:line:col message` — 坏文件 ERROR 级
    assert!(
        texts.iter().any(|t| t.starts_with("error ") && t.contains("broken.lua")),
        "诊断行 `error <file>…`\n实际 texts={texts:?}"
    );

    // ---- 阶段 D: live search — 输入 query 即过滤 (spec: as the user types) ----
    set_input(handle, &view, InputSlot::Query, "on_ready", cx);
    let texts = render_texts(handle, cx);
    assert!(
        texts.iter().any(|t| t.starts_with("on_ready fn ")),
        "query 命中须在场: {texts:?}"
    );
    // 反向: 不匹配的 query → 无命中行
    set_input(handle, &view, InputSlot::Query, "zzz_no_such_symbol", cx);
    let texts = render_texts(handle, cx);
    assert!(
        !texts.iter().any(|t| t.contains(" fn ")),
        "不匹配 query 须无命中行: {texts:?}"
    );

    // 收尾复位 (不污染同进程其它测试)。
    set_roots(Vec::new());
    let _ = std::fs::remove_dir_all(&dir);
}
