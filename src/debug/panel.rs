//! debug-console change task 5.2: debug section 的签包装。
//!
//! 注册表工厂产出 section 视图 Entity (design D2), 本 Panel 把它包成
//! 中槽签: `closable=true`, `on_removed` 摘 section dedup (task 5.1),
//! `title()` 直读登记标题。视图经 [`AnyView`] 类型擦除持有 — gpui 官方
//! 注释明言其"participates in the reactive graph exactly like any other
//! view", 反应性 (notify → 重绘) 无损。

use gpui_kit::base::dock::{PanelEvent, PanelId};
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::dock::{BasePanel, Panel as ComponentPanel};
use gpui_kit::{
    AnyView, App, Context, EventEmitter, Focusable, FocusHandle, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, Styled, WeakEntity, Window, div,
};

use crate::app::TerminalApp;

/// debug section 签面板: 持类型擦除的 section 视图。
pub struct DebugSectionPanel {
    /// section 视图 (类型擦除的 Entity; 反应图无损)。
    view: AnyView,
    /// tab 标题 = 登记的 `title` (`title()` 直读, 测试经此断言)。
    pub(crate) title: SharedString,
    /// 弱引用 app: `on_removed` 摘 section dedup。
    app: WeakEntity<TerminalApp>,
    /// 自身面板身份 (= entity_id 派生), 供摘表比对。
    panel_id: PanelId,
    /// 面板焦点 handle (签切换交焦目标)。
    focus: FocusHandle,
}

impl DebugSectionPanel {
    pub fn new(
        title: &str,
        view: AnyView,
        app: WeakEntity<TerminalApp>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            view,
            title: title.into(),
            app,
            panel_id: PanelId::from(cx.entity_id()),
            focus: cx.focus_handle(),
        }
    }
}

impl BasePanel for DebugSectionPanel {
    fn panel_name(&self) -> &'static str {
        "debug-section"
    }
    fn closable(&self, _: &App) -> bool {
        true // specs/center-tabs delta: debug section 签可关
    }
    /// 签被关闭 (或被整体置换) → 摘 section dedup, 同 section 可重开
    /// (specs/center-tabs: Closing a tab removes its entry)。
    fn on_removed(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let app = self.app.clone();
        let panel_id = self.panel_id;
        app.update(cx, |app, _| app.debug_tab_removed(panel_id)).ok();
    }
}

impl ComponentPanel for DebugSectionPanel {
    fn title(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().text_sm().child(self.title.clone())
    }
    fn title_bar(&self, _: &App) -> bool {
        false // 与 viewer 同款: 面板自带 chrome; 多签时 tab bar 画全部签
    }
    fn inner_padding(&self, _: &App) -> bool {
        false // section 视图自控布局
    }
}

impl EventEmitter<PanelEvent> for DebugSectionPanel {}

impl Focusable for DebugSectionPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for DebugSectionPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("debug-section-pane")
            // 注册面板焦点 handle 参与焦点链 (md_panel 同款 — 签切换交焦
            // 后 key dispatch 走它; section 视图内部焦点自理)
            .track_focus(&self.focus)
            .size_full()
            .bg(cx.theme().background)
            .child(self.view.clone())
            .into_any_element()
    }
}

#[cfg(test)]
#[path = "panel_tests.rs"]
mod tests;
