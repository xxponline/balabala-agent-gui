//! InteractiveFrame: 一个扩展的 egui::Frame, 负责:
//!   1. 根据 active 状态切换边框颜色 (active 蓝 / inactive 灰)
//!   2. 点击 Frame 任意位置 → request_focus
//!   3. 拥有 focus 时声明 output.ime, 让 egui-winit 打开 macOS IME
//!   4. 拥有 focus 时, 把 Event::Ime(Commit(s)) 通过闭包转发出去
//!
//! 用法:
//! ```ignore
//! InteractiveFrame::new()
//!     .fill(Color32::from_rgb(30, 30, 30))
//!     .inner_margin(Margin::same(8))
//!     .show(ui, active, &mut |s| { /* IME commit */ }, |ui| {
//!         // contents
//!     });
//! ```

use eframe::egui::{self, Color32, Id, Margin, Stroke};

const ACTIVE_COLOR: Color32 = Color32::from_rgb(75, 161, 215);
const INACTIVE_COLOR: Color32 = Color32::from_rgb(60, 60, 60);

fn frame_id() -> Id {
    Id::unique("interactive_frame")
}

pub struct InteractiveFrame {
    fill: Color32,
    inner_margin: Margin,
}

impl InteractiveFrame {
    pub fn new() -> Self {
        Self {
            fill: Color32::from_rgb(30, 30, 30),
            inner_margin: Margin::same(0),
        }
    }

    pub fn fill(mut self, color: Color32) -> Self {
        self.fill = color;
        self
    }

    pub fn inner_margin(mut self, margin: Margin) -> Self {
        self.inner_margin = margin;
        self
    }

    /// 渲染一帧。
    ///
    /// - `active`: 当前是否是选中的面板 (决定边框颜色)
    /// - `on_commit`: IME 上屏时调用, 参数是上屏的 UTF-8 字符串
    /// - `contents`: 闭包里画真正的内容
    pub fn show<R>(
        self,
        ui: &mut egui::Ui,
        active: bool,
        on_commit: &mut dyn FnMut(&str),
        contents: impl FnOnce(&mut egui::Ui) -> R,
    ) -> R {
        let stroke_color = if active { ACTIVE_COLOR } else { INACTIVE_COLOR };

        let id = ui.make_persistent_id("interactive_frame");

        // 用 max_rect 确保覆盖整个 pane 区域
        let rect = ui.max_rect();
        let response = ui.interact(rect, id, egui::Sense::click());
        if response.clicked() {
            response.request_focus();
        }

        let has_focus = ui.memory(|m| m.focused() == Some(id));

        if has_focus {
            ui.output_mut(|o| {
                o.ime = Some(egui::output::IMEOutput {
                    purpose: egui::IMEPurpose::Normal,
                    rect,
                    cursor_rect: rect,
                    should_interrupt_composition: false,
                });
            });

            let events = ui.input(|i| i.raw.events.clone());
            for ev in events {
                if let egui::Event::Ime(egui::ImeEvent::Commit(s)) = ev {
                    if !s.is_empty() {
                        on_commit(&s);
                    }
                }
            }
        }

        egui::Frame::new()
            .fill(self.fill)
            .stroke(Stroke::new(1.5, stroke_color))
            .inner_margin(self.inner_margin)
            .show(ui, contents)
            .inner
    }
}

impl Default for InteractiveFrame {
    fn default() -> Self {
        Self::new()
    }
}
