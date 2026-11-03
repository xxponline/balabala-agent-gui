//! 主 App 状态: 持有 workspaces 列表, 调度 egui 的 logic() / ui()。

use std::path::PathBuf;

use eframe::egui;

use crate::agent_entities::PiAgentPty;
use crate::ui::{pane, sidebar};
use crate::workspace::{Workspace, store};

pub struct TerminalApp {
    /// 所有 workspace (每个持有一个路径 + 该路径下的 agents)
    pub workspaces: Vec<Workspace>,
    /// 当前在 main 区显示的 (workspace_idx, agent_idx)
    pub active: Option<(usize, usize)>,
    /// 最近一次持久化失败的错误信息 (在 sidebar 顶部显示)
    pub save_error: Option<String>,
}

impl TerminalApp {
    /// 启动时从 `.workspaces.json` 读取, 失败时返回 Err 让 main 退出。
    pub fn new() -> Result<Self, String> {
        let paths = store::load()?;
        let workspaces = paths
            .into_iter()
            .map(Workspace::new)
            .collect::<Vec<_>>();
        Ok(Self {
            workspaces,
            active: None,
            save_error: None,
        })
    }

    /// 把当前 workspaces 的路径列表刷到 `.workspaces.json`,
    /// 失败时把错误存到 self.save_error (而不是 panic)。
    fn persist(&mut self) {
        let paths: Vec<PathBuf> = self.workspaces.iter().map(|w| w.path.clone()).collect();
        match store::save(&paths) {
            Ok(_) => {
                self.save_error = None;
            }
            Err(e) => {
                self.save_error = Some(e);
            }
        }
    }

    /// 用户点了 "[+ new workspace]": 弹 rfd 文件夹选择器。
    fn create_workspace(&mut self) {
        let picked = rfd::FileDialog::new().pick_folder();
        let Some(path) = picked else {
            return; // 用户取消
        };

        // canonicalize 方便去重
        let path = std::fs::canonicalize(&path).unwrap_or(path);

        // 去重
        if self.workspaces.iter().any(|w| w.path == path) {
            eprintln!("workspace already exists: {}", path.display());
            return;
        }

        self.workspaces.push(Workspace::new(path));
        self.persist();
    }

    /// 用户点了某个 workspace 下的 "[+ agent]"。
    fn create_agent(&mut self, ws_idx: usize) {
        if ws_idx >= self.workspaces.len() {
            return;
        }
        let ws = &mut self.workspaces[ws_idx];
        let name = ws.new_agent_name();
        let cwd = ws.path.clone();

        match PiAgentPty::new(name, "pi".to_string(), cwd) {
            Ok(agent) => {
                self.workspaces[ws_idx].agents.push(Box::new(agent));
                let agent_idx = self.workspaces[ws_idx].agents.len() - 1;
                self.active = Some((ws_idx, agent_idx));
            }
            Err(e) => {
                eprintln!("failed to spawn agent: {e}");
            }
        }
    }

    /// 用户点了 workspace 的 "×": 只在 workspace 为空时删除。
    fn close_workspace(&mut self, ws_idx: usize) {
        if ws_idx >= self.workspaces.len() {
            return;
        }
        if !self.workspaces[ws_idx].agents.is_empty() {
            return; // 还有 agent, 禁止删除
        }
        self.workspaces.remove(ws_idx);

        // 调整 active
        self.active = match self.active {
            None => None,
            Some((active_ws, _active_agent)) if active_ws == ws_idx => None,
            Some((active_ws, active_agent)) if active_ws > ws_idx => {
                Some((active_ws - 1, active_agent))
            }
            Some(x) => Some(x),
        };

        self.persist();
    }

    /// 用户点了某个 agent 的 "×"。
    fn close_agent(&mut self, ws_idx: usize, agent_idx: usize) {
        if ws_idx >= self.workspaces.len() {
            return;
        }
        let ws = &mut self.workspaces[ws_idx];
        if agent_idx >= ws.agents.len() {
            return;
        }
        ws.agents[agent_idx].shutdown();
        ws.agents.remove(agent_idx);

        // 调整 active
        self.active = match self.active {
            None => None,
            Some((a_ws, a_agent)) if a_ws == ws_idx && a_agent == agent_idx => {
                // 关掉的是 active: 切到同 workspace 内不超过新长度的最后一个
                if ws.agents.is_empty() {
                    None
                } else {
                    Some((ws_idx, ws.agents.len() - 1))
                }
            }
            Some((a_ws, a_agent)) if a_ws == ws_idx && a_agent > agent_idx => {
                Some((a_ws, a_agent - 1))
            }
            Some(x) => Some(x),
        };
    }

    /// 把键盘事件翻译成字节送给 active agent。
    fn forward_keyboard(&mut self, ctx: &egui::Context) {
        let Some((ws_idx, agent_idx)) = self.active else {
            return;
        };
        if ws_idx >= self.workspaces.len() {
            return;
        }
        if agent_idx >= self.workspaces[ws_idx].agents.len() {
            return;
        }

        let events = ctx.input(|i| i.raw.events.clone());
        for ev in events {
            match ev {
                egui::Event::Text(s) => {
                    self.workspaces[ws_idx].agents[agent_idx].send_input(s.as_bytes());
                }
                // Event::Ime 由 InteractiveFrame 处理, 这里不匹配
                egui::Event::Key {
                    key, pressed: true, ..
                } => match key {
                    egui::Key::Enter => self.workspaces[ws_idx].agents[agent_idx].send_input(b"\r"),
                    egui::Key::Backspace => {
                        self.workspaces[ws_idx].agents[agent_idx].send_input(b"\x7f")
                    }
                    egui::Key::Tab => self.workspaces[ws_idx].agents[agent_idx].send_input(b"\t"),
                    egui::Key::ArrowUp => {
                        self.workspaces[ws_idx].agents[agent_idx].send_input(b"\x1b[A")
                    }
                    egui::Key::ArrowDown => {
                        self.workspaces[ws_idx].agents[agent_idx].send_input(b"\x1b[B")
                    }
                    egui::Key::ArrowRight => {
                        self.workspaces[ws_idx].agents[agent_idx].send_input(b"\x1b[C")
                    }
                    egui::Key::ArrowLeft => {
                        self.workspaces[ws_idx].agents[agent_idx].send_input(b"\x1b[D")
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }
}

impl eframe::App for TerminalApp {
    fn logic(&mut self, _ctx: &egui::Context, _frame: &mut eframe::Frame) {
        for ws in self.workspaces.iter_mut() {
            for agent in ws.agents.iter_mut() {
                agent.poll();
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.forward_keyboard(ui.ctx());

        ui.horizontal(|ui| {
            // ---- 左边栏 ----
            ui.with_layout(
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_min_width(160.0);
                    ui.set_max_width(260.0);
                    let save_error = self.save_error.as_deref();
                    let action = sidebar::render(ui, &self.workspaces, self.active, save_error);

                    if action.new_workspace {
                        self.create_workspace();
                    }
                    if let Some(ws_idx) = action.toggle_collapse {
                        if let Some(ws) = self.workspaces.get_mut(ws_idx) {
                            ws.collapsed = !ws.collapsed;
                        }
                    }
                    if let Some(ws_idx) = action.close_workspace {
                        self.close_workspace(ws_idx);
                    }
                    if let Some(ws_idx) = action.new_agent {
                        self.create_agent(ws_idx);
                    }
                    if let Some(pair) = action.select_agent {
                        self.active = Some(pair);
                    }
                    if let Some((ws_idx, agent_idx)) = action.close_agent {
                        self.close_agent(ws_idx, agent_idx);
                    }
                },
            );

            ui.separator();

            // ---- 右边主区 ----
            // 强制占满窗口高度, 不被 sidebar 压扁
            let screen_h = ui.ctx().content_rect().height();
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_min_height(screen_h);
                let active_pair = self.active;

                if active_pair.is_none() {
                    pane::render_empty(ui);
                    return;
                }

                let (ws_idx, agent_idx) = active_pair.unwrap();

                // 先拿只读 view 数据 (status bar + vt100 渲染需要)
                // 注意: 这里只借 &self.workspaces, 不创建可变闭包
                let view = self.workspaces
                    .get(ws_idx)
                    .and_then(|w| w.agents.get(agent_idx))
                    .map(|a| {
                        (
                            a.ws_path().to_path_buf(),
                            a.name().to_string(),
                            a.status(),
                            a.screen() as *const vt100::Screen,
                        )
                    });

                let Some((path_str, name, status, screen_ptr)) = view else {
                    pane::render_empty(ui);
                    return;
                };

                // 现在创建可变 on_commit 闭包
                let workspaces = &mut self.workspaces;
                let mut on_commit = |s: &str| {
                    if let Some(agent) = workspaces
                        .get_mut(ws_idx)
                        .and_then(|w| w.agents.get_mut(agent_idx))
                    {
                        agent.send_input(s.as_bytes());
                    }
                };

                // 把 screen 指针转回引用 (safty: agent 还在 workspaces 里,
                // 我们只是临时绕开 borrow checker)
                let screen = unsafe { &*screen_ptr };

                // 恢复正常的 pane 渲染
                pane::render_active(ui, &mut on_commit, |ui| {
                    let (rows, cols) = screen.size();
                    let (rows, cols) = (rows.get(), cols.get());
                    let (cur_row, cur_col) = screen.cursor_position();
                    let path_str = path_str.to_string_lossy().to_string();

                    // 顶部状态条
                    ui.horizontal(|ui| {
                        ui.colored_label(
                            egui::Color32::from_rgb(75, 161, 215),
                            format!("● {path_str} — {name}"),
                        );
                        ui.label(
                            egui::RichText::new(format!("[{status:?}]"))
                                .color(egui::Color32::from_rgb(150, 150, 150)),
                        );
                    });
                    ui.separator();

                    if status == crate::agent_entities::SessionStatus::Starting {
                        ui.label(
                            egui::RichText::new("waiting for pi to start...")
                                .color(egui::Color32::from_rgb(229, 193, 16))
                                .monospace(),
                        );
                    }

                    // vt100 网格
                    ui.vertical(|ui| {
                        ui.style_mut().override_font_id =
                            Some(egui::FontId::monospace(13.0));

                        for r in 0..rows {
                            let mut segments: Vec<(String, egui::Color32, egui::Color32)> =
                                Vec::new();
                            let mut cur_text = String::new();
                            let mut cur_fg = egui::Color32::WHITE;
                            let mut cur_bg = egui::Color32::from_rgb(30, 30, 30);

                            for c in 0..cols {
                                let cell = match screen.cell(r, c) {
                                    Some(cell) => cell,
                                    None => continue,
                                };
                                let mut fg = crate::ui::fg_to_egui(cell.fgcolor());
                                let mut bg = crate::ui::bg_to_egui(cell.bgcolor());

                                if r == cur_row && c == cur_col {
                                    std::mem::swap(&mut fg, &mut bg);
                                }

                                if (fg, bg) != (cur_fg, cur_bg) && !cur_text.is_empty() {
                                    segments.push((
                                        std::mem::take(&mut cur_text),
                                        cur_fg,
                                        cur_bg,
                                    ));
                                    cur_fg = fg;
                                    cur_bg = bg;
                                }
                                cur_text.push_str(cell.contents());
                            }
                            if !cur_text.is_empty() {
                                segments.push((cur_text, cur_fg, cur_bg));
                            }

                            ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                            ui.horizontal(|ui| {
                                for (text, fg, bg) in segments {
                                    if bg == egui::Color32::from_rgb(30, 30, 30) {
                                        ui.colored_label(fg, text);
                                    } else {
                                        egui::Frame::new()
                                            .fill(bg)
                                            .show(ui, |ui| {
                                                ui.colored_label(fg, text);
                                            });
                                    }
                                }
                            });
                        }
                    });
                });
            });
        });

        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(50));
    }
}
