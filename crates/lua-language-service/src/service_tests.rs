//! service 核心无头契约 (tasks 2.1–2.5)。
//!
//! 纯 `#[test]` — service 不依赖 gpui。状态是**进程级单例**: 所有测试
//! (含后续 view 测试) 持 [`TEST_LOCK`] 串行, fixture 写 pid 隔离的临时目录。
//!
//! 断言纪律 (抗负载 flake, c4b1365 教训): 优先**语义断言** (代际/状态机
//! 可推导的必然), 时序只作兜底且给宽界。

use std::path::PathBuf;
use std::sync::MutexGuard;
use std::time::{Duration, Instant};

use super::{TEST_LOCK, diagnostics_all, index_status, search_symbols, set_roots, wait_settled};
use crate::types::{DiagnosticSeverity, IndexPhase, SymbolKind};

fn guard() -> MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

/// 写 fixture 目录 (pid 隔离), 返回根路径。`files` = (相对路径, 内容)。
fn write_fixture(tag: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lua-ls-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (rel, content) in files {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().expect("fixture parent")).expect("mkdir fixture");
        std::fs::write(&path, content).expect("write fixture");
    }
    dir
}

/// 批量 fixture: `m/f0000.lua …`, 内容由闭包生成 (后台/部分状态测试用)。
fn write_fixture_many(tag: &str, n: usize, content: impl Fn(usize) -> String) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lua-ls-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("m")).expect("mkdir fixture");
    for i in 0..n {
        std::fs::write(dir.join(format!("m/f{i:04}.lua")), content(i)).expect("write fixture");
    }
    dir
}

fn cleanup(dir: &PathBuf) {
    let _ = std::fs::remove_dir_all(dir);
}

// ---------- 2.1 全量替换 ----------

#[test]
fn set_roots_full_replace_and_clear() {
    let _g = guard();
    let a = write_fixture(
        "repl-a",
        &[("lib.lua", "function alpha_fn()\nend\n")],
    );
    let b = write_fixture(
        "repl-b",
        &[("lib.lua", "function beta_fn()\nend\n")],
    );

    set_roots(vec![a.clone()]);
    let st = wait_settled();
    assert_eq!(st.phase, IndexPhase::Idle, "A 索引落定: {st:?}");
    assert_eq!(st.file_count, 1, "A 全部文件计入");
    assert!(st.elapsed_ms.is_some(), "elapsed 落定: {st:?}");
    assert_eq!(st.roots, vec![a.clone()], "状态记录注入 roots");
    assert!(
        search_symbols("alpha").iter().any(|h| h.name == "alpha_fn"),
        "A 符号可搜出"
    );

    // 全量替换的**同步半边**: set_roots 返回即刻旧 roots 不可见
    // (spec scenario: Injecting a new root set drops the old one)。
    set_roots(vec![b.clone()]);
    assert!(
        search_symbols("alpha").is_empty(),
        "替换后旧符号须同步消失 (不等索引)"
    );

    let st = wait_settled();
    assert_eq!(st.phase, IndexPhase::Idle, "B 索引落定: {st:?}");
    assert_eq!(st.roots, vec![b.clone()]);
    assert!(
        search_symbols("beta").iter().any(|h| h.name == "beta_fn"),
        "B 符号可搜出"
    );
    assert!(
        search_symbols("alpha").is_empty(),
        "旧 roots 符号不得残留"
    );

    // 空注入 = 清空一切 (spec scenario: Empty injection clears everything)。
    set_roots(vec![]);
    let st = index_status();
    assert_eq!(st.phase, IndexPhase::Empty, "空注入即刻 Empty");
    assert!(st.roots.is_empty());
    assert_eq!(st.file_count, 0);
    assert!(st.elapsed_ms.is_none());
    assert!(search_symbols("beta").is_empty(), "空注入后搜索为空");
    assert!(diagnostics_all().is_empty(), "空注入后诊断为空");

    cleanup(&a);
    cleanup(&b);
}

// ---------- 2.2 后台索引 + 状态 ----------

#[test]
fn indexing_runs_in_background() {
    let _g = guard();
    let n = 120;
    let dir = write_fixture_many("bg", n, |i| {
        format!("local function bg_sym_{i}()\n  return {i}\nend\n")
    });

    set_roots(vec![dir.clone()]);
    // set_roots 只做状态复位 + 投递 — 建库 (init_std_lib + fs 收集) 本身
    // 即 ≥ms 级, 远超返回与本读取间的 µs 级间隙 → 同步索引必露馅。
    let st = index_status();
    assert_eq!(
        st.phase,
        IndexPhase::InProgress,
        "set_roots 返回后须处于后台索引中 (非同步)"
    );
    assert!(st.file_count < n, "文件计数须在增长中而非已完成: {st:?}");

    let st = wait_settled();
    assert_eq!(st.phase, IndexPhase::Idle, "后台完成: {st:?}");
    assert_eq!(st.file_count, n, "全部文件计入 file_count");
    assert!(st.elapsed_ms.is_some(), "elapsed 在索引完成时落定");

    cleanup(&dir);
}

// ---------- 2.3 符号搜索 ----------

#[test]
fn symbol_search_answers_by_name() {
    let _g = guard();
    let dir = write_fixture(
        "search",
        &[
            ("src/net/client.lua", "function on_ready()\nend\n"),
            (
                "src/util.lua",
                "function on_ready_extra()\nend\nlocal helper = 1\n",
            ),
        ],
    );
    set_roots(vec![dir.clone()]);
    let st = wait_settled();
    assert_eq!(st.phase, IndexPhase::Idle, "{st:?}");

    // 前缀查询 (spec scenario: Prefix query returns definitions)。
    let hits = search_symbols("on_");
    let h = hits
        .iter()
        .find(|x| x.name == "on_ready")
        .expect("on_ready 须命中");
    assert_eq!(h.kind, SymbolKind::Function, "`function` 声明分类为 Function");
    assert!(
        h.file.ends_with("src/net/client.lua"),
        "命中定位到定义文件: {:?}",
        h.file
    );
    // "function " 9 字符 → 名字 0-based col 9 → 1-based col 10, 第 1 行。
    assert_eq!((h.line, h.col), (1, 10), "1-based 行列");

    // 精确命中排在前缀命中之前 (排序契约)。
    let hits = search_symbols("on_ready");
    assert_eq!(
        hits.first().map(|x| x.name.as_str()),
        Some("on_ready"),
        "精确命中须优先于前缀命中: {hits:?}"
    );
    assert!(
        hits.iter().any(|x| x.name == "on_ready_extra"),
        "前缀命中仍在场"
    );

    // 无 workspace → 空 (spec scenario: Search with no workspace returns empty)。
    set_roots(vec![]);
    assert!(search_symbols("on_").is_empty());

    cleanup(&dir);
}

// ---------- 2.4 诊断 + 陈旧扫描中止 ----------

#[test]
fn workspace_diagnostics_and_stale_abort() {
    let _g = guard();

    // --- 段 1: 语法错误 → 诊断 (severity + message) ---
    let bad = write_fixture("diag-bad", &[("broken.lua", "local x = (\n")]);
    set_roots(vec![bad.clone()]);
    let st = wait_settled();
    assert_eq!(st.phase, IndexPhase::Idle, "{st:?}");
    let diags = diagnostics_all();
    assert!(!diags.is_empty(), "语法错误须产出诊断");
    let d = diags
        .iter()
        .find(|d| d.file.ends_with("broken.lua"))
        .expect("诊断定位到坏文件");
    assert_eq!(
        d.severity,
        DiagnosticSeverity::Error,
        "SyntaxError 默认 ERROR 级: {d:?}"
    );
    assert!(!d.message.is_empty(), "诊断须带消息: {d:?}");

    // --- 段 2: 空 workspace → 空 (spec scenario) ---
    set_roots(vec![]);
    assert!(diagnostics_all().is_empty(), "空注入即刻清空诊断");

    // --- 段 3: 陈旧 sweep 中止 (spec: a new set_roots during a sweep aborts) ---
    let n = 120;
    let a = write_fixture_many("stale-a", n, |i| {
        format!("local function stale_sym_{i}()\nend\nlocal x = (\n")
    });
    let b = write_fixture("stale-b", &[("only.lua", "local z = (\n")]);

    set_roots(vec![a.clone()]);
    // 等 A 索引完成 (file_count==n) — 此刻 sweep 可能在跑, 也可能已结束;
    // 两条路径都合法, 后续断言与之无关。
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let st = index_status();
        if st.file_count == n {
            break;
        }
        if matches!(st.phase, IndexPhase::Idle | IndexPhase::Failed) {
            break; // 已落定 (file_count 亦应 ==n)
        }
        assert!(Instant::now() < deadline, "等 A 索引超时: {st:?}");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        !search_symbols("stale_sym_").is_empty(),
        "接管前 A 符号可搜 (证明其本在场)"
    );

    // 接管: A 的 job 处于 sweep 中或已落定 — 无论哪条路径,
    // set_roots(B) 同步清诊断 + 代际递增 → A 的陈旧结果永不落地。
    set_roots(vec![b.clone()]);
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        // 轮询全程: 任何时刻不得出现 A 的诊断 (spec: 陈旧扫描结果不落地)。
        for d in diagnostics_all() {
            assert!(
                d.file.starts_with(&b),
                "陈旧诊断落地 (代际守卫失效): {d:?}"
            );
        }
        let st = index_status();
        if matches!(st.phase, IndexPhase::Idle | IndexPhase::Failed) {
            break;
        }
        assert!(Instant::now() < deadline, "等 B 落定超时: {st:?}");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        search_symbols("stale_sym_").is_empty(),
        "接管后 A 符号不得残留"
    );
    let diags = diagnostics_all();
    assert!(!diags.is_empty(), "B 自身诊断须落定");
    for d in &diags {
        assert!(d.file.starts_with(&b), "诊断只属 B: {d:?}");
    }

    cleanup(&bad);
    cleanup(&a);
    cleanup(&b);
}

// ---------- 2.5 索引中查询 (部分状态) ----------

#[test]
fn query_during_indexing_reads_partial_state() {
    let _g = guard();
    let n = 400;
    let dir = write_fixture_many("partial", n, |i| {
        format!("local function part_sym_{i}()\nend\n")
    });
    set_roots(vec![dir.clone()]);

    // 等"部分状态"窗口: 已索引 >0 且不到一半 (剩余工作量足量)。
    let deadline = Instant::now() + Duration::from_secs(60);
    let partial = loop {
        let st = index_status();
        if st.file_count > 0 && st.file_count * 2 < n {
            break Some(st.file_count);
        }
        if matches!(st.phase, IndexPhase::Idle | IndexPhase::Failed) {
            // 窗口被调度延迟吃掉 (极端负载) → 降级为仅断言落定, 不误报。
            break None;
        }
        assert!(Instant::now() < deadline, "等部分索引超时: {st:?}");
        std::thread::sleep(Duration::from_millis(1));
    };

    if let Some(count) = partial {
        let t = Instant::now();
        let hits = search_symbols("part_sym_");
        let cost = t.elapsed();
        let st = index_status();
        if st.file_count < n || st.phase == IndexPhase::InProgress {
            // 索引仍在进行而查询已返回 → 语义上证明未等待整轮完成。
        } else {
            // 查询恰逢索引收尾 → 时序兜底 (宽松界, 抗负载)。
            assert!(
                cost < Duration::from_millis(500),
                "查询不得等待整轮索引: {cost:?}"
            );
        }
        assert!(
            !hits.is_empty(),
            "部分状态须可搜出已索引符号 (当时 file_count={count})"
        );
    }

    let st = wait_settled();
    assert_eq!(st.phase, IndexPhase::Idle, "{st:?}");
    assert_eq!(st.file_count, n);
    assert_eq!(
        search_symbols("part_sym_").len(),
        n,
        "落定后全量符号可搜"
    );

    cleanup(&dir);
}
