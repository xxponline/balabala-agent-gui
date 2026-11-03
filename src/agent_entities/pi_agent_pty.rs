//! `PiAgentPty`: portable-pty + vt100 + pi 子进程 的具体实现。
//!
//! 生命周期:
//!   new(cmd)  --spawn-->  child + reader thread
//!   poll()   <--drain--  mpsc<AgentEvent>
//!   send_input(bytes)  --> master writer --> child stdin
//!   shutdown()  --> child.kill()
//!
//! 状态机:
//!   Starting (刚 spawn, 没收到任何 Output)
//!     | 第一次收到 Output
//!     v
//!   Running
//!     | 收到 Exited / child 退出
//!     v
//!   Exited

use std::io::{Read, Write};
use std::num::NonZeroU16;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::thread;

use portable_pty::{Child, CommandBuilder, PtySize, native_pty_system};
use vt100::Parser;

use crate::agent_entities::{AgentEntity, AgentEvent, SessionStatus};

/// 终端网格写死的尺寸 (resize 同步在 pending-decisions.md 里)
const COLS: u16 = 100;
const ROWS: u16 = 30;

pub struct PiAgentPty {
    /// sidebar 上显示的名字, 例如 "pi-1"
    name: String,
    /// agent 启动时的工作目录 (workspace 路径)
    ws_path: PathBuf,
    /// 子进程句柄, 用于 kill
    child: Box<dyn Child + Send + Sync>,
    /// master 写端: 把用户输入送给子进程 stdin
    writer: Box<dyn Write + Send>,
    /// vt100 解析器: UI 直接读它的 screen
    parser: Parser,
    /// reader 线程 -> UI 线程 的事件通道
    event_rx: Receiver<AgentEvent>,
    /// 当前状态
    status: SessionStatus,
}

impl PiAgentPty {
    /// 创建一个新的 pty 会话并 spawn `cmd`, 在 `cwd` 目录下运行。
    ///
    /// `name` 是 sidebar 显示的名字 (由 app 层负责分配)。
    pub fn new(name: String, cmd: String, cwd: PathBuf) -> Result<Self, String> {
        // ---- 1. 开 pty ----
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: ROWS,
                cols: COLS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("openpty: {e}"))?;

        // ---- 2. spawn 子进程 ----
        let mut cmd_builder = CommandBuilder::new(&cmd);
        // 强制让子进程以为自己在真终端里, 否则很多程序不输出颜色
        cmd_builder.env("TERM", "xterm-256color");
        // 让 pi 在 workspace 指定的目录下运行
        cmd_builder.cwd(cwd.clone());
        let child = pair
            .slave
            .spawn_command(cmd_builder)
            .map_err(|e| format!("spawn {cmd}: {e}"))?;

        // ---- 3. 拿 master 的读写两端 ----
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| format!("take_writer: {e}"))?;

        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("clone reader: {e}"))?;

        // ---- 4. reader 线程: 把字节流封装成 AgentEvent 发给 UI ----
        let (tx, rx) = std::sync::mpsc::channel::<AgentEvent>();
        thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => {
                        // EOF: 子进程关闭了 stdout
                        let _ = tx.send(AgentEvent::Exited);
                        break;
                    }
                    Ok(n) => {
                        if tx.send(AgentEvent::Output(buf[..n].to_vec())).is_err() {
                            break; // UI 线程已经挂了
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => {
                        let _ = tx.send(AgentEvent::Exited);
                        break;
                    }
                }
            }
        });

        // ---- 5. vt100 parser ----
        let parser = Parser::new(
            NonZeroU16::new(ROWS).unwrap(),
            NonZeroU16::new(COLS).unwrap(),
            1000,
        );

        Ok(Self {
            name,
            ws_path: cwd,
            child,
            writer,
            parser,
            event_rx: rx,
            status: SessionStatus::Starting,
        })
    }
}

impl AgentEntity for PiAgentPty {
    fn name(&self) -> &str {
        &self.name
    }

    fn ws_path(&self) -> &Path {
        &self.ws_path
    }

    fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }

    fn poll(&mut self) {
        // drain 所有事件, 更新 parser + status
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                AgentEvent::Output(bytes) => {
                    self.parser.process(&bytes);
                    if self.status == SessionStatus::Starting {
                        self.status = SessionStatus::Running;
                    }
                }
                AgentEvent::Exited => {
                    self.status = SessionStatus::Exited;
                }
            }
        }
    }

    fn send_input(&mut self, bytes: &[u8]) {
        if self.writer.write_all(bytes).is_ok() {
            let _ = self.writer.flush();
        }
    }

    fn shutdown(&mut self) {
        let _ = self.child.kill();
        self.status = SessionStatus::Exited;
    }

    fn status(&self) -> SessionStatus {
        self.status
    }
}
