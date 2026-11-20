//! 只读 `.rs` 查看面板（design D4; specs/center-tabs: Read-only Rust
//! viewer handler）。
//!
//! gpui-component 内建 `Editor` 只读展示 + syntect 高亮（`gpui-kit/syntect`
//! feature; 不启用则黑白纯文本）。**不自研**只读渲染：白得选区/复制/滚动/
//! 行号；保存系统明确 Non-Goal。

use std::path::Path;

use gpui_kit::base::dock::{PanelEvent, PanelId, PanelView};
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::input::{Editor, EditorState};
use gpui_kit::component::dock::panel_handle;
use gpui_kit::{
    App, AppContext, Context, Entity, EventEmitter, Focusable, FocusHandle, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, Styled, TestSupportExt, WeakEntity, Window,
    div,
};

use crate::app::TerminalApp;
use crate::property_detail_manager::{OpenHandler, OpenObject};
use crate::assets::AssetKind;
use gpui_kit::component::dock::BasePanel;
use gpui_kit::component::dock::Panel as ComponentPanel;

/// 只读 `.rs` 查看面板实体。
pub struct EditorPanel {
    /// 编辑器状态（readonly 置于状态侧 + 视图侧双保险, design D4）。
    pub(crate) state: Entity<EditorState>,
    /// tab 标题 = 文件名（`title()` 直读, 测试经此断言）。
    pub(crate) title: SharedString,
    /// 弱引用 app: `on_removed` 摘 dedup（design D5）。
    app: WeakEntity<TerminalApp>,
    /// 自身面板身份（= entity_id 派生的 PanelId）, 供摘表比对。
    panel_id: PanelId,
}

impl EditorPanel {
    /// 读文件灌入只读 Editor。文件不可读 → 空 buffer（打开不失败, 与
    /// "unclaimed object opens nothing" 不冲突: 认领已发生, 只是内容为空）。
    pub fn new(
        obj: &OpenObject,
        app: WeakEntity<TerminalApp>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let content = std::fs::read_to_string(&obj.path).unwrap_or_default();
        let language = language_for(&obj.path);
        let state = cx.new(|cx| {
            // default_value 而非 replace: replace 会把光标移到替换文本
            // 末尾并 scroll_to(cursor) — 打开即停在文件末尾 (headed 验收
            // 实测)。default_value 直接置 Rope (prepare 时才进 display_map),
            // 光标/滚动保持初始 0, 首行即文件开头。
            let mut state = EditorState::new(window, cx)
                .language(language)
                .default_value(content);
            state.set_readonly(true, cx);
            state
        });
        let panel_id = PanelId::from(cx.entity_id());
        Self {
            state,
            title: file_title(&obj.path),
            app,
            panel_id,
        }
    }
}

/// v1: 仅 `rs` 认领（proposal 定案; 放宽 = predicate 一行改）。
fn claims_rust(obj: &OpenObject) -> bool {
    obj.kind == AssetKind::File
        && obj.path.extension().is_some_and(|ext| ext == "rs")
}

/// Editor 语言名（v1 只有 rust; 与认领表同步扩展）。
fn language_for(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs") => "rust",
        _ => "plaintext",
    }
}

/// tab 标题 = 文件名（无文件名时退回 lossy 全路径）。
fn file_title(path: &Path) -> SharedString {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
        .into()
}

/// 首个 handler: `.rs` → [`EditorPanel`]（specs/center-tabs 首个 handler）。
pub struct RustViewerHandler;

impl OpenHandler for RustViewerHandler {
    fn claims(&self, obj: &OpenObject) -> bool {
        claims_rust(obj)
    }

    fn create_panel(
        &self,
        obj: &OpenObject,
        window: &mut Window,
        cx: &mut Context<TerminalApp>,
    ) -> std::sync::Arc<dyn PanelView> {
        let app = cx.weak_entity();
        let panel = cx.new(|cx| EditorPanel::new(obj, app, window, cx));
        panel_handle(panel)
    }
}

impl BasePanel for EditorPanel {
    fn panel_name(&self) -> &'static str {
        "viewer"
    }
    fn closable(&self, _: &App) -> bool {
        true // viewer 签可关（specs/center-tabs: 签关闭与去重表生命周期）
    }
    /// 签被关闭（或被整体置换）→ 摘 dedup, 保证同对象可重开
    ///（design D5; specs/center-tabs: Closing a tab removes its entry）。
    fn on_removed(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let app = self.app.clone();
        let panel_id = self.panel_id;
        app.update(cx, |app, _| app.open_tab_removed(panel_id)).ok();
    }
}

impl ComponentPanel for EditorPanel {
    fn title(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().text_sm().child(self.title.clone())
    }
    fn title_bar(&self, _: &App) -> bool {
        false // 与 console 同款 chrome 策略 (design D8 同源)
    }
    fn inner_padding(&self, _: &App) -> bool {
        false
    }
}

impl EventEmitter<PanelEvent> for EditorPanel {}

impl Focusable for EditorPanel {
    /// 焦点 = EditorState 的焦点 handle — tab 切换的 `focus_active_panel`
    /// 与点击都落到这里, 键盘/IME 随之跟到 viewer（spec: Keystrokes follow
    /// the active tab）。
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.state.read(cx).focus_handle(cx)
    }
}

impl Render for EditorPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let editor = Editor::new(&self.state)
            .readonly(true)
            .bordered(false)
            // 拉满面板高度 (headed 验收: 默认按内容行数拥抱 → “非常短”且
            // 无滚动视口, 无法向上翻阅; h_full 后超限内容走 Editor 滚动)
            .h_full();
        div()
            .id("viewer-pane")
            .test_support() // 观测锚点: press/input 的 scope focus 断言
            .size_full()
            .bg(cx.theme().background)
            .child(editor)
            .into_any_element()
    }
}
