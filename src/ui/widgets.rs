//! 共享 UI 小件 (design R1 收尾; tasks 5.4)。
//!
//! 无业务状态的跨模块复用件归位于此; `ui/mod.rs` 保留颜色映射 (theme)。

use gpui_kit::component::ActiveTheme;
use gpui_kit::*;

/// tooltip 里的纯文本 view (tooltip builder 需要返回 AnyView)。
pub struct TextTooltip {
    pub text: String,
}

impl Render for TextTooltip {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .rounded_md()
            .bg(cx.theme().background)
            .border_1()
            .border_color(cx.theme().border)
            .text_xs()
            .child(self.text.clone())
    }
}
