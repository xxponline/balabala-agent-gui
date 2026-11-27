//! `lua-index` debug section 视图 (design D1: view 住 crate, 主 crate 只接线)。
//!
//! **构造无 window** (factory 签名 `fn(&str, &mut Context<TerminalApp>)`,
//! design D1 的三行 adapter) — `InputState::new` 需 window, 故输入实体在
//! **首帧渲染时惰性创建** (render 拿得到 `&mut Window`), 仅一次。
//!
//! **渲染直读服务**: 每次 render 同步读 `index_status/search_symbols/
//! diagnostics_all` — 1s tick 只负责在后台状态变化时 `notify` 触发重绘,
//! 输入框 observe 变化即 notify (spec: as the user types, 无显式提交)。
//! 实体亡 → tick Task 随之取消 (关签即停, 同其它 section)。

use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::base::TestTextExt;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::{
    AppContext, AsyncApp, Context, Entity, InteractiveElement, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, Subscription, Task, Window, div,
};

use crate::service::{diagnostics_all, index_status, search_symbols, set_roots};
use crate::types::{IndexStatus, LuaDiagnostic, SymbolHit};

/// 命中/诊断行渲染上限 — 调试视图防巨量结果拖垮布局 (超出显示截断行)。
const ROW_LIMIT: usize = 100;

pub struct LuaIndexView {
    /// root 注入输入 (首帧惰性建, 见文件头)。
    root_input: Option<Entity<InputState>>,
    /// 符号搜索输入 (同上; 变化即 notify → 重绘重搜)。
    query_input: Option<Entity<InputState>>,
    /// 输入变化订阅 (存活即语义 — drop 即退订)。
    _subs: Vec<Subscription>,
    /// 1s tick: 后台索引/诊断落定 → notify 重绘 (实体亡即停)。
    _task: Task<()>,
    /// browse… 对话框任务 (存续期保活, drop 即取消 — solution_config 同款)。
    _browse_task: Option<Task<()>>,
}

impl LuaIndexView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let task = cx.spawn(async move |this, cx: &mut AsyncApp| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_secs(1))
                    .await;
                let Some(view) = this.upgrade() else {
                    break; // 实体亡 (关签) — 任务止
                };
                view.update(cx, |_, cx| cx.notify());
            }
        });
        Self {
            root_input: None,
            query_input: None,
            _subs: Vec::new(),
            _task: task,
            _browse_task: None,
        }
    }

    /// browse…: 系统默认目录选择框 → 选中即**填输入 + 注入** (调试视图
    /// 一步到位; solution_config 的"只填不交"是其 spec 决策, 此处无此约束)。
    /// 取消 picker → 早退, 输入与 workspace 均不变 (同款取消语义)。
    fn browse(&mut self, cx: &mut Context<Self>) {
        let task = cx.spawn(async move |view, cx: &mut AsyncApp| {
            let Some(handle) = rfd::AsyncFileDialog::new().pick_folder().await else {
                return; // 取消: 不改输入不注入
            };
            let picked = handle.path().to_path_buf();
            let path = std::fs::canonicalize(&picked).unwrap_or(picked);
            let text = path.to_string_lossy().into_owned();
            view.update(cx, |view, cx| {
                let entity_id = cx.entity().entity_id();
                let input = view.root_input.clone();
                let inject = path.clone();
                // update_in 需 VisualContext (AsyncWindow 等专用上下文),
                // Context<T> 走 with_window 拿 window + &mut App 直改
                // (solution_config browse 同款)。
                cx.with_window(entity_id, move |window, cx| {
                    if let Some(input) = input {
                        input.update(cx, |state, cx| state.set_value(text, window, cx));
                    }
                });
                // 选完即注入 — 与 inject 按钮同一 set_roots 入口。
                set_roots(vec![inject]);
                cx.notify();
            })
            .ok();
        });
        self._browse_task = Some(task);
    }

    /// 首帧一次性建输入 + 订阅查询框变化 (spec: live search, 无显式提交)。
    fn ensure_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.root_input.is_some() {
            return;
        }
        let root = cx.new(|cx| InputState::new(window, cx));
        let query = cx.new(|cx| InputState::new(window, cx));
        let watched = query.clone();
        self._subs
            .push(cx.observe(&watched, |_, _, cx| cx.notify()));
        self.root_input = Some(root);
        self.query_input = Some(query);
    }

    /// 注入按钮/测试缝: 读 root 输入 → 全量替换注入 (空输入 = 清空)。
    pub fn inject(&mut self, cx: &mut Context<Self>) {
        let Some(input) = self.root_input.clone() else {
            return; // 未渲染过 (不可达: 按钮只在渲染后存在)
        };
        let text = input.read(cx).value().to_string();
        let trimmed = text.trim();
        set_roots(if trimmed.is_empty() {
            Vec::new()
        } else {
            vec![PathBuf::from(trimmed)]
        });
        cx.notify(); // 立即反映 InProgress/Empty
    }
}

// ---------- 行文 (渲染与测试同源 — term_rate rows() 惯例) ----------

/// 状态行: `files=N elapsed=Xms phase=Idle [error=…]`。
fn status_row(s: &IndexStatus) -> String {
    let elapsed = s
        .elapsed_ms
        .map(|m| format!("{m}ms"))
        .unwrap_or_else(|| "-".to_string());
    let err = s
        .error
        .as_ref()
        .map(|e| format!(" error={e}"))
        .unwrap_or_default();
    format!(
        "files={} elapsed={} phase={:?}{}",
        s.file_count, elapsed, s.phase, err
    )
}

/// roots 行: 当前注入集 (spec: 状态含注入 roots)。
fn roots_row(s: &IndexStatus) -> String {
    if s.roots.is_empty() {
        "roots=(none)".to_string()
    } else {
        let joined = s
            .roots
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(",");
        format!("roots={joined}")
    }
}

/// 命中行: `name kind file:line:col` (spec: name, kind, file, line, column)。
fn hit_row(h: &SymbolHit) -> String {
    format!(
        "{} {} {}:{}:{}",
        h.name,
        h.kind.label(),
        h.file.display(),
        h.line,
        h.col
    )
}

/// 诊断行: `severity file:line:col message` (spec: file, position, severity, message)。
fn diag_row(d: &LuaDiagnostic) -> String {
    format!(
        "{} {}:{}:{} {}",
        d.severity.label(),
        d.file.display(),
        d.line,
        d.col,
        d.message
    )
}

/// 单行观测口 (test_text 双挂 — pane/tree 同款)。
fn row(text: String) -> gpui_kit::AnyElement {
    div().child(text.clone()).test_text(text).into_any_element()
}

impl Render for LuaIndexView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_inputs(window, cx);

        // 直读服务 (渲染期同步读; 查询读锁, 最坏等一批索引 — D5)。
        let status = index_status();
        let query = self
            .query_input
            .as_ref()
            .map(|q| q.read(cx).value().to_string())
            .unwrap_or_default();
        let hits = search_symbols(&query);
        let diagnostics = diagnostics_all();

        let root_input = self
            .root_input
            .clone()
            .expect("ensure_inputs 已建 root 输入");
        let query_input = self
            .query_input
            .clone()
            .expect("ensure_inputs 已建 query 输入");
        let on_inject = cx.listener(|view, _ev, _window, cx| view.inject(cx));
        let on_browse = cx.listener(|view, _ev, _window, cx| view.browse(cx));

        let status_row = status_row(&status);
        let roots_row = roots_row(&status);
        let hits_head = format!("hits={}", hits.len());
        let hit_rows: Vec<String> = hits.iter().take(ROW_LIMIT).map(hit_row).collect();
        let hits_more = hits.len().saturating_sub(ROW_LIMIT);
        let diag_head = format!("diagnostics={}", diagnostics.len());
        let diag_rows: Vec<String> = diagnostics.iter().take(ROW_LIMIT).map(diag_row).collect();
        let diags_more = diagnostics.len().saturating_sub(ROW_LIMIT);

        let theme_border = cx.theme().border;

        // 状态/roots 行 (spec: file count / elapsed / phase)。
        let head_rows = vec![row(status_row), row(roots_row)];
        // 命中 + 诊断行 (可选截断行)。
        let mut tail_rows: Vec<gpui_kit::AnyElement> = vec![row(hits_head)];
        tail_rows.extend(hit_rows.into_iter().map(row));
        if hits_more > 0 {
            tail_rows.push(row(format!("... {hits_more} more hits")));
        }
        tail_rows.push(row(diag_head));
        tail_rows.extend(diag_rows.into_iter().map(row));
        if diags_more > 0 {
            tail_rows.push(row(format!("... {diags_more} more diagnostics")));
        }

        div()
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .children(head_rows)
            // root 注入控制 (spec: root path injection control)。
            .child(
                div()
                    .flex()
                    .gap_1()
                    .child(div().child("root").test_text("root-label"))
                    .child(Input::new(&root_input).w_full())
                    .child(
                        div()
                            .id("lua-index-inject")
                            .cursor_pointer()
                            .px_2()
                            .py_1()
                            .border_1()
                            .rounded_md()
                            .border_color(theme_border)
                            .on_click(on_inject)
                            .child(div().child("inject").test_text("inject")),
                    )
                    .child(
                        div()
                            .id("lua-index-browse")
                            .cursor_pointer()
                            .px_2()
                            .py_1()
                            .border_1()
                            .rounded_md()
                            .border_color(theme_border)
                            .on_click(on_browse)
                            .child(div().child("browse…").test_text("browse…")),
                    ),
            )
            // 搜索输入 (spec: live search as the user types)。
            .child(
                div()
                    .flex()
                    .gap_1()
                    .child(div().child("search").test_text("search-label"))
                    .child(Input::new(&query_input).w_full()),
            )
            // 命中列表 + 诊断表 (spec: hits list / workspace diagnostics)。
            .children(tail_rows)
    }
}

#[cfg(test)]
#[path = "view_tests.rs"]
mod tests;
