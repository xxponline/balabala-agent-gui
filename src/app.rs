//! 主 App 状态: 持有 agent 列表, 调度 egui 的 logic() / ui()。

use eframe::egui;

use crate::agent_entities::{AgentEntity, PiAgentPty};
use crate::ui::{pane, sidebar};

pub struct TerminalApp {
    /// 当前所有 agent session
    pub agents: Vec<Box<dyn AgentEntity>>,
    /// 当前在 main 区显示的 agent 下标 (None = 空 main 区)
    pub active_id: Option<usize>,
    /// 下一个自动编号: pi-{next_name_idx}
    pub next_name_idx: u32,
}

impl TerminalApp {
    pub fn new() -> Self {
        Self {
            agents: Vec::new(),
            active_id: None,
            next_name_idx: 1,
        }
    }

    /// 点 "+": 新建一个 pi agent 并立刻 active
    fn create_new_agent(&mut self) {
        let name = format!("pi-{}", self.next_name_idx);
        self.next_name_idx += 1;

        // 默认 spawn "pi" (Q10 = A: 裸 pi, 无参数)
        match PiAgentPty::new(name, "pi".to_string()) {
            Ok(agent) => {
                self.agents.push(Box::new(agent));
                self.active_id = Some(self.agents.len() - 1);
            }
            Err(e) => {
                // spawn 失败 (比如 pi 不在 PATH): 打个 println, 不 panic
                eprintln!("failed to spawn agent: {e}");
            }
        }
    }

    /// 关闭某个 agent, 必要时调整 active_id
    fn close_agent(&mut self, idx: usize) {
        if idx >= self.agents.len() {
            return;
        }
        self.agents[idx].shutdown();
        self.agents.remove(idx);

        // 调整 active_id: 如果删掉的就是 active, 切到下一个 (或 None)
        match self.active_id {
            None => {}
            Some(active) if idx == active => {
                // 关掉的是 active: 切到不超过新长度的最后一个
                self.active_id = if self.agents.is_empty() {
                    None
                } else if idx >= self.agents.len() {
                    Some(self.agents.len() - 1)
                } else {
                    Some(idx)
                };
            }
            Some(active) if idx < active => {
                // 删掉的在 active 前面, 下标要往左挪一位
                self.active_id = Some(active - 1);
            }
            Some(_) => {
                // 删掉的在 active 后面, active 不变
            }
        }
    }

    /// 7.x: 把键盘事件翻译成字节送给 active agent
    fn forward_keyboard(&mut self, ctx: &egui::Context) {
        let Some(active_idx) = self.active_id else {
            return;
        };
        if active_idx >= self.agents.len() {
            return;
        }

        let events = ctx.input(|i| i.raw.events.clone());
        for ev in events {
            match ev {
                // 7.1: 可打印字符 (包括 IME 提交的字符串)
                egui::Event::Text(s) => {
                    self.agents[active_idx].send_input(s.as_bytes());
                }
                // 7.2: 控制键翻译成 VT100 转义序列
                egui::Event::Key {
                    key, pressed: true, ..
                } => match key {
                    egui::Key::Enter => self.agents[active_idx].send_input(b"\r"),
                    egui::Key::Backspace => self.agents[active_idx].send_input(b"\x7f"),
                    egui::Key::Tab => self.agents[active_idx].send_input(b"\t"),
                    egui::Key::ArrowUp => self.agents[active_idx].send_input(b"\x1b[A"),
                    egui::Key::ArrowDown => self.agents[active_idx].send_input(b"\x1b[B"),
                    egui::Key::ArrowRight => self.agents[active_idx].send_input(b"\x1b[C"),
                    egui::Key::ArrowLeft => self.agents[active_idx].send_input(b"\x1b[D"),
                    _ => {}
                },
                _ => {}
            }
        }
    }
}

impl eframe::App for TerminalApp {
    /// 每帧调用, 但不能画 UI。适合把 channel 里的新字节喂给 parser。
    fn logic(&mut self, _ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 8.2: 遍历所有 agent 调 poll()
        for agent in self.agents.iter_mut() {
            agent.poll();
        }
    }

    /// 每帧重绘: 画 sidebar + main pane。
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // ---- 7.x: 先把键盘事件送给 active agent ----
        self.forward_keyboard(ui.ctx());

        // ---- 左右分栏: sidebar | main pane ----
        ui.horizontal(|ui| {
            // 左边栏 (固定窄宽)
            ui.with_layout(
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_min_width(160.0);
                    ui.set_max_width(220.0);
                    let action = sidebar::render(ui, &self.agents, self.active_id);

                    // 处理 sidebar 动作
                    if action.clicked_new {
                        self.create_new_agent();
                    }
                    if let Some(idx) = action.selected {
                        self.active_id = Some(idx);
                    }
                    if let Some(idx) = action.closed {
                        self.close_agent(idx);
                    }
                },
            );

            ui.separator();

            // 右边主区
            ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| {
                // 把 active agent 借出来给 pane 渲染
                let active = self
                    .active_id
                    .and_then(|idx| self.agents.get_mut(idx));
                pane::render(ui, active);
            });
        });

        // 8.4: 异步刷新, 保证 pty 输出能持续更新
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(50));
    }
}
