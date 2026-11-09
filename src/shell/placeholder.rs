//! 右 dock 占位面板（design §5.3 / tasks 5.1）：**下个功能模块的接入样板**。
//!
//! chrome: header（标题 + `×`）+ 居中提示 body。`×` → 关闭右 dock
//! （`toggle_dock(Right)`，经 `TerminalApp::close_right_dock`）——
//! 关闭后焦点显式交回终端面板（spike §10.5/E2：点 `×` 会把焦点交给
//! 即将隐藏的面板，不交回则键盘/IME 悬空）。
//!
//! 本轮**无会话内重开路径**（grill 定案：重启恢复默认）；下个功能
//! 替换本文件即接入，dock 机制（resize/open/注册）零改动。

use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent};
use gpui_kit::*;

use crate::app::TerminalApp;

pub struct RightPlaceholderPanel {
    app: WeakEntity<TerminalApp>,
    /// `Focusable` 契约要求；dock 交互时会聚焦此 handle（spike E2 实测）。
    focus: FocusHandle,
}

impl RightPlaceholderPanel {
    pub fn new(app: WeakEntity<TerminalApp>, cx: &mut Context<Self>) -> Self {
        Self {
            app,
            focus: cx.focus_handle(),
        }
    }
}

impl BasePanel for RightPlaceholderPanel {
    fn panel_name(&self) -> &'static str {
        "right-placeholder"
    }
    fn closable(&self, _: &App) -> bool {
        false // 关的是 dock 不是 panel（ClosePanel 会移除面板, 不用）
    }
    fn zoomable(&self, _: &App) -> bool {
        false
    }
}

impl Panel for RightPlaceholderPanel {
    fn title_bar(&self, _: &App) -> bool {
        false // D8: 自带 header chrome
    }
    fn inner_padding(&self, _: &App) -> bool {
        false
    }
}

impl EventEmitter<PanelEvent> for RightPlaceholderPanel {}

impl Focusable for RightPlaceholderPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for RightPlaceholderPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let app_refocus = self.app.clone();
        div()
            .id("right-root")
            .test_support() // 观测锚点 (无 test-support 特性时为 no-op)
            .size_full()
            .flex()
            .flex_col()
            // 占位面板不抢键盘 (spec: Keyboard input forwarding — 窗口聚焦
            // 即转发): dock 交互会把焦点交给本面板, 点击任意处交回终端
            // (spike E2 同款恢复路径; `×` 的处理器也交回, 幂等)。
            .on_click(cx.listener(move |_, _event: &ClickEvent, window, cx| {
                let app = app_refocus.clone();
                app.update(cx, |app, cx| {
                    let focus = app.terminal.read(cx).focus_handle(cx);
                    focus.focus(window, cx);
                })
                .ok();
            }))
            .child(
                // header: 标题 + 关闭钮（下个功能的 chrome 样板）
                div()
                    .id("right-header")
                    .test_support() // 观测锚点
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .py_1()
                    .border_b_1()
                    .border_color(rgb(0xd9d9d9))
                    .child(div().text_sm().child("right dock"))
                    .child(
                        div()
                            .id("right-close")
                            .cursor_pointer()
                            .px_1()
                            .text_sm()
                            .hover(|style| style.text_color(rgb(0xcd3131)))
                            .child("×")
                            .on_click(cx.listener(
                                |this, _event: &ClickEvent, window, cx| {
                                    let app = this.app.clone();
                                    // toggle + 焦点交回终端面板 (spike E2)
                                    app.update(cx, |app, cx| {
                                        app.close_right_dock(window, cx)
                                    })
                                    .ok();
                                },
                            )),
                    ),
            )
            .child(
                div()
                    .id("right-body")
                    .flex_1()
                    .w_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_1()
                    .text_sm()
                    .text_color(rgb(0x969696))
                    .child("next feature panel")
                    .child("下一个功能模块将填充此面板"),
            )
    }
}
