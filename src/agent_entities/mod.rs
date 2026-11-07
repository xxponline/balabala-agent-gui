//! AgentEntity trait 与相关类型。
//!
//! 任何能在 main pane 里渲染的 "agent 会话" 都要实现这个 trait。
//! 目前的具体实现是 `PiAgentShell` (进程内嵌入 pi-shell-rs; 原 PTY 实现
//! `PiAgentPty` 已在 embed-pi-shell change 中移除, 见 git history),
//! 未来可以加 mock / remote 等。

use std::path::Path;

use vt100::Screen;

pub mod pi_agent_shell;
pub use pi_agent_shell::PiAgentShell;

/// gpui entity 包装: shell 以 `Entity<AgentSession>` 持有会话,
/// 通过 `read(cx)` / `update(cx)` 安全访问 (design D2)。
pub struct AgentSession(pub Box<dyn AgentEntity>);

/// sidebar 上显示的状态徽章 (spec: Sidebar session item display)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStatus {
    /// 会话已创建, 还没有输出
    Starting,
    /// 已有输出, 会话连接仍然打开
    Running,
    /// 会话连接已关闭 (pi 退出或已关闭)
    Exited,
}

/// 一个可被渲染 / 可被注入输入的 agent 会话
pub trait AgentEntity {
    /// sidebar 上显示的名字, 例如 "pi-1"
    fn name(&self) -> &str;

    /// agent 启动时的工作目录 (workspace 路径)
    fn ws_path(&self) -> &Path;

    /// 当前 vt100 屏幕 (UI 直接读 cell(r, c))
    fn screen(&self) -> &Screen;

    /// 消费新产出的渲染帧并更新 status / parser。
    /// 返回 true 表示本次消费了内容 (UI 据此决定是否 notify)。
    /// 由 shell 的节流循环每 50ms 调用。
    fn poll(&mut self) -> bool;

    /// 把一个键送给会话。
    fn send_key(&mut self, key: crossterm::event::KeyEvent);

    /// 把一段已提交文本送给会话（IME commit；preedit 不走这里）。
    fn send_text(&mut self, text: &str);

    /// 主动关闭: 结束会话并终止 pi 子进程 (spec: Manual session close)。
    fn shutdown(&mut self);

    /// 当前状态 (Starting / Running / Exited)
    fn status(&self) -> SessionStatus;
}
