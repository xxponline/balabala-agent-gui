//! md 只读查看面板（change: md-viewer-toggle, design D1-D6）。
//!
//! 双模式单面板（D1）：Rendered = `TextView::markdown` 纯静态渲染、Source =
//! 复用 Editor 管线（syntect `markdown` 语法）。两个重量级 state 构造期
//! 一次建好（D3），切换只换渲染分支，不重建 state、不重读文件（D5）。
//!
//! 静态渲染已源码验证（design Context）：全量替换路径立即显示、Motion 默认
//! 全零，构造侧 `.stream_fade(false)` 兜底 theme 时序。

use std::path::Path;

use gpui_kit::base::dock::{PanelEvent, PanelId, PanelView};
use gpui_kit::component::dock::panel_handle;
use gpui_kit::component::dock::BasePanel;
use gpui_kit::component::dock::Panel as ComponentPanel;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::input::{Editor, EditorState};
use gpui_kit::component::text::{TextView, TextViewState};
use gpui_kit::{
    App, AppContext, ClickEvent, Context, Entity, EventEmitter, Focusable, FocusHandle,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString, Styled, TestSupportExt,
    WeakEntity, Window, div,
};

use crate::app::TerminalApp;
use crate::property_detail_manager::{OpenHandler, OpenObject};
use asset_service::contract::AssetKind;

/// 查看模式（design D1; 默认 Rendered — proposal 定案, 不记忆）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ViewMode {
    /// 渲染视图：markdown 排版展示（纯静态, Non-Goal 不做节点扩展）。
    Rendered,
    /// 源码视图：原始 markdown + 语法高亮的只读编辑器。
    Source,
}

/// md 查看面板实体。
pub struct MdPanel {
    /// Rendered 态（构造期建好, 切换不重建 — design D3）。
    pub(crate) md_state: Entity<TextViewState>,
    /// Source 态（同上; readonly 双保险于状态侧 + 视图侧）。
    pub(crate) editor: Entity<EditorState>,
    /// 当前模式（默认 Rendered; 每次打开重置 — 不记忆）。
    pub(crate) view_mode: ViewMode,
    /// tab 标题 = 文件名（`title()` 直读, 测试经此断言）。
    pub(crate) title: SharedString,
    /// Rendered 态的焦点 handle（Source 态转发给 Editor — design D4）。
    focus: FocusHandle,
    /// 弱引用 app: `on_removed` 摘 dedup（design D5）。
    app: WeakEntity<TerminalApp>,
    /// 自身面板身份（= entity_id 派生的 PanelId）, 供摘表比对。
    panel_id: PanelId,
}

impl MdPanel {
    /// 读文件灌入双态 state。文件不可读 → 空 buffer（打开不失败, 与
    /// EditorPanel 同语义: 认领已发生, 只是内容为空）。
    pub fn new(
        obj: &OpenObject,
        app: WeakEntity<TerminalApp>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let content = std::fs::read_to_string(&obj.path).unwrap_or_default();
        // Rendered 态: 构造即全文（replacement 路径, 立即显示 — 验证结论 2）。
        let md_state = cx.new(|cx| TextViewState::markdown(&content, cx));
        // Source 态: default_value 而非 replace（首行在顶 — EditorPanel 同款
        // 教训）。
        let editor = cx.new(|cx| {
            let mut state = EditorState::new(window, cx)
                .language("markdown")
                .default_value(content);
            state.set_readonly(true, cx);
            state
        });
        let panel_id = PanelId::from(cx.entity_id());
        Self {
            md_state,
            editor,
            view_mode: ViewMode::Rendered,
            title: file_title(&obj.path),
            focus: cx.focus_handle(),
            app,
            panel_id,
        }
    }

    /// 切换模式（design D1: 改 enum + notify; title_suffix 按钮与
    /// ToggleMdView action 共用此方法体）。
    fn toggle(&mut self, cx: &mut Context<Self>) {
        self.view_mode = match self.view_mode {
            ViewMode::Rendered => ViewMode::Source,
            ViewMode::Source => ViewMode::Rendered,
        };
        cx.notify();
    }
}

/// md 认领（proposal: default-open 认领表扩展 `md`）。
fn claims_markdown(obj: &OpenObject) -> bool {
    obj.kind == AssetKind::File
        && obj.path.extension().is_some_and(|ext| ext == "md")
}

/// tab 标题 = 文件名（无文件名时退回 lossy 全路径）。
fn file_title(path: &Path) -> SharedString {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
        .into()
}

/// md handler: `.md` → [`MdPanel`]（specs/center-tabs: Read-only markdown
/// viewer handler）。
pub struct MarkdownViewerHandler;

impl OpenHandler for MarkdownViewerHandler {
    fn claims(&self, obj: &OpenObject) -> bool {
        claims_markdown(obj)
    }

    fn create_panel(
        &self,
        obj: &OpenObject,
        window: &mut Window,
        cx: &mut Context<TerminalApp>,
    ) -> std::sync::Arc<dyn PanelView> {
        let app = cx.weak_entity();
        let panel = cx.new(|cx| MdPanel::new(obj, app, window, cx));
        panel_handle(panel)
    }
}

impl BasePanel for MdPanel {
    fn panel_name(&self) -> &'static str {
        "md-viewer"
    }
    fn closable(&self, _: &App) -> bool {
        true // viewer 签可关（与 EditorPanel 同: 关签摘 dedup 可重开）
    }
    /// 签被关闭 → 摘 dedup, 保证同对象可重开（specs/center-tabs:
    /// Closing a tab removes its entry）。
    fn on_removed(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let app = self.app.clone();
        let panel_id = self.panel_id;
        app.update(cx, |app, _| app.open_tab_removed(panel_id)).ok();
    }
}

impl ComponentPanel for MdPanel {
    fn title(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().text_sm().child(self.title.clone())
    }
    fn title_bar(&self, _: &App) -> bool {
        false // 与 console/viewer 同款 chrome 策略 (D8 同源)
    }
    fn inner_padding(&self, _: &App) -> bool {
        false
    }

    /// 切换按钮（spec: Markdown view toggle）。`title_suffix` 是 dock 的
    /// 单一挂点, 两种 tab 形态同路: 单签标题栏尾部（tab_panel.rs:430）与
    /// 多签 tab 条尾部（同文件:695, 仅 active panel 提供）— 条件显示天然
    /// 成立: console/`.rs` viewer 返回 None, 无按钮（spec 场景 3）。
    /// 按钮 label = 目标模式（当前渲染 → 显示"源码"）。
    fn title_suffix(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        let target = match self.view_mode {
            ViewMode::Rendered => "源码",
            ViewMode::Source => "渲染",
        };
        // Sizable::xsmall 在 gpui-component 私有模块（0.9.0 未 re-export,
        // 外部不可达）→ 先用默认尺寸, headed 验收看视觉再调。
        let button = gpui_kit::component::button::Button::new("md-toggle")
            .child(target)
            .on_click(cx.listener(
                |this, _: &ClickEvent, _window, cx| this.toggle(cx),
            ));
        Some(button)
    }
}

impl EventEmitter<PanelEvent> for MdPanel {}

impl Focusable for MdPanel {
    /// 焦点按 mode 转发（design D4）: Source → EditorState（与 EditorPanel
    /// 同语义）; Rendered → 面板自身 handle（只读浏览）。
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        match self.view_mode {
            ViewMode::Rendered => self.focus.clone(),
            ViewMode::Source => self.editor.read(cx).focus_handle(cx),
        }
    }
}

impl Render for MdPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        match self.view_mode {
            ViewMode::Rendered => div()
                .id("md-pane")
                .test_support() // 观测锚点: scope focus 断言（两分支都要）
                // 注册面板 focus handle 参与焦点链 (press/input 的 observed
                // focus 与 key dispatch 都走它 — Editor 内部自 track, 同理)
                .track_focus(&self.focus)
                .size_full()
                // 左右边距 (headed 验收 1: 渲染模式完全无边距)。
                .px_4()
                .bg(cx.theme().background)
                // action 入口（tasks 3.1）: 与 title_suffix 按钮同 body。
                .on_action(cx.listener(|this, _: &ToggleMdView, _window, cx| {
                    this.toggle(cx)
                }))
                // 保险行: 静态全文走 replacement 路径本无淡入, 此行兜底
                // theme 层配置的 stream_fade 时序（design 验证结论 3）。
                // scrollable(true) (headed 验收 2: 默认 false 不可滚 — 大内容
                // 溢出; 要求父容器固定高度, size_full 满足)。
                .child(TextView::new(&self.md_state).stream_fade(false).scrollable(true))
                .into_any_element(),
            ViewMode::Source => {
                let editor = Editor::new(&self.editor)
                    .readonly(true)
                    .bordered(false)
                    .h_full(); // 超限内容走 Editor 滚动（EditorPanel 同款）
                div()
                    .id("md-source-pane")
                    .test_support()
                    .size_full()
                    .bg(cx.theme().background)
                    .on_action(cx.listener(|this, _: &ToggleMdView, _window, cx| {
                        this.toggle(cx)
                    }))
                    .child(editor)
                    .into_any_element()
            }
        }
    }
}

// ToggleMdView action（tasks 3.1: 只挂 MdPanel 元素树, console/rs 无响应）。
// `actions!` 宏生成 unit action（gpui-kit facade: 不依赖 gpui 原名）;
// doc 注释不能挂宏调用, 语义记于此。
gpui_kit::actions!(md_view, [ToggleMdView]);
