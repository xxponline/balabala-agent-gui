//! `PiAgentShell`: 进程内嵌入 pi-shell-rs 的 AgentEntity 实现（design D1/D2）。
//!
//! 生命周期:
//!   new(name, cwd) --EmbeddedApp::spawn--> pi --mode rpc 子进程 (带 cwd)
//!   poll()         <--take_ansi--        embed 帧 (ANSI) -> vt100 parser
//!   send_key/send_text --> embed 注入通道 (UiEvent::Key)
//!   shutdown()     --> embed.shutdown()  (退出事件循环 + 杀 pi 子进程)
//!
//! 状态机 (spec: Sidebar session item display):
//!   Starting (会话已创建, 尚无帧)
//!     | 第一帧 ANSI
//!     v
//!   Running  (embed 线程存活)
//!     | embed 线程退出 (pi 自行退出 / 关闭)
//!     v
//!   Exited   (最后一帧保留在 vt100 屏幕上)
//!
//! 与原 PTY 实现的对照关系见 design D6: `screen()` 契约不变,
//! 输入从字节改为结构化 KeyEvent / 文本。

use std::num::NonZeroU16;
use std::path::{Path, PathBuf};

use pi_shell_rs::app::embed::{EmbedConfig, EmbeddedApp};
use vt100::Parser;

use crate::agent_entities::{AgentEntity, SessionStatus};

/// 终端网格尺寸, 与 pane.rs 的渲染网格一致 (与原 PTY 实现相同的常量)。
const COLS: u16 = 100;
const ROWS: u16 = 30;

/// vt100 scrollback 行数 (与原 PTY 实现一致)。
const SCROLLBACK: usize = 1000;

pub struct PiAgentShell {
    /// sidebar 上显示的名字, 例如 "pi-1"
    name: String,
    /// agent 启动时的工作目录 (workspace 路径)
    ws_path: PathBuf,
    /// 嵌入会话句柄 (输入注入 / 取帧 / 状态 / 关闭)
    embed: EmbeddedApp,
    /// vt100 解析器: embed ANSI 帧 -> 屏幕 (UI 直接读 cell)
    parser: Parser,
    /// 当前状态
    status: SessionStatus,
}

impl PiAgentShell {
    /// 创建嵌入会话并在 `cwd` (workspace 路径) 下启动 pi。
    ///
    /// `name` 是 sidebar 显示的名字 (由 app 层负责分配)。
    /// 失败 (pi 缺失/探活超时) 返回 Err, 不会留下半开的会话。
    pub fn new(name: String, cwd: PathBuf) -> Result<Self, String> {
        let config = EmbedConfig {
            pi_bin: None, // resolve_binary(): PI_BIN 覆盖或 PATH 里的 pi
            pi_args: Vec::new(),
            cwd: Some(cwd.clone()),
            cols: COLS,
            rows: ROWS,
            // 标题/剪贴板: GUI 尚未接线 (见 change 跟进项)。显式 no-op,
            // 避免缺省 sink 把 SetTitle/OSC52 转义序列写进 GUI 进程的 stdout。
            set_title: Some(Box::new(|_t: String| {})),
            copy_text: Some(Box::new(|_t: String| {})),
        };
        let embed = EmbeddedApp::spawn(config)?;

        let parser = Parser::new(
            NonZeroU16::new(ROWS).unwrap(),
            NonZeroU16::new(COLS).unwrap(),
            SCROLLBACK,
        );

        Ok(Self {
            name,
            ws_path: cwd,
            embed,
            parser,
            status: SessionStatus::Starting,
        })
    }
}

impl AgentEntity for PiAgentShell {
    fn name(&self) -> &str {
        &self.name
    }

    fn ws_path(&self) -> &Path {
        &self.ws_path
    }

    fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }

    fn poll(&mut self) -> bool {
        // 1) 吃掉 embed 产出的 ANSI 帧, 喂 vt100 parser。
        let bytes = self.embed.take_ansi();
        if !bytes.is_empty() {
            self.parser.process(&bytes);
            if self.status == SessionStatus::Starting {
                self.status = SessionStatus::Running;
            }
            return true; // 有消费 -> UI notify
        }
        // 2) embed 线程退出 (pi 自行退出/连接关闭) -> Exited;
        //    最后一帧保留在屏幕上 (spec: Observed subprocess exit)。
        if !self.embed.is_running() && self.status != SessionStatus::Exited {
            self.status = SessionStatus::Exited;
            return true;
        }
        false
    }

    fn send_key(&mut self, key: crossterm::event::KeyEvent) {
        self.embed.inject_key(key);
    }

    fn send_text(&mut self, text: &str) {
        self.embed.inject_text(text);
    }

    fn shutdown(&mut self) {
        // 退出事件循环并 join (线程侧杀 pi 子进程, spec: Manual session close)
        self.embed.shutdown();
        self.status = SessionStatus::Exited;
    }

    fn status(&self) -> SessionStatus {
        self.status
    }
}
