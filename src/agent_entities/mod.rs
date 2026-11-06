//! AgentEntity trait 与相关类型。
//!
//! 任何能在 main pane 里渲染的 "agent 会话" 都要实现这个 trait。
//! 目前的具体实现是 `PiAgentPty` (pty + vt100 + pi 子进程),
//! 未来可以加 mock / remote 等。

use std::num::NonZeroU16;
use std::path::Path;
use std::sync::mpsc::Receiver;

use vt100::Screen;

pub mod pi_agent_pty;
pub use pi_agent_pty::PiAgentPty;

/// gpui entity 包装: shell 以 `Entity<AgentSession>` 持有会话,
/// 通过 `read(cx)` / `update(cx)` 安全访问 (design D2)。
pub struct AgentSession(pub Box<dyn AgentEntity>);

/// reader 线程 -> UI 线程 的事件类型
#[derive(Debug)]
pub enum AgentEvent {
    /// 子进程产出新字节, 需要喂给 vt100 parser
    Output(Vec<u8>),
    /// 子进程 stdout EOF (退出)
    Exited,
}

/// sidebar 上显示的状态徽章
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStatus {
    /// 刚 spawn, 还没收到任何输出
    Starting,
    /// 收到过输出, 子进程还活着
    Running,
    /// 子进程已经退出
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

    /// 把 reader channel 里的新事件吃掉, 顺便更新 status / parser。
    /// 返回 true 表示本次 drain 消费了事件 (UI 据此决定是否 notify)。
    /// 由 shell 的节流循环每 50ms 调用。
    fn poll(&mut self) -> bool;

    /// 把用户键盘输入的字节送回子进程 stdin
    fn send_input(&mut self, bytes: &[u8]);

    /// 主动关闭: hard kill 子进程
    fn shutdown(&mut self);

    /// 当前状态 (Starting / Running / Exited)
    fn status(&self) -> SessionStatus;
}

// 防止 unused 警告在 stub 阶段; 真正的实现文件会用到
#[allow(dead_code)]
fn _unused_helpers() -> (NonZeroU16, Receiver<AgentEvent>) {
    (
        NonZeroU16::new(1).unwrap(),
        std::sync::mpsc::channel().1,
    )
}
