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

/// 终端网格回退尺寸 (首帧测量前使用; 自适应后由 pane 测量接管 —
/// 见 change `add-adaptive-terminal-grid` design D6)。与 pane.rs 渲染一致。
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
    /// 当前网格尺寸 (cols, rows): 去重依据; 初始为回退常量 (D6),
    /// 首帧测量后由 `resize` 接管
    grid_size: (u16, u16),
    /// 当前状态
    status: SessionStatus,
}

impl PiAgentShell {
    /// 创建嵌入会话并在 `cwd` (workspace 路径) 下启动 pi。
    ///
    /// `name` 是 sidebar 显示的名字 (由 app 层负责分配)。
    /// 失败 (pi 缺失/探活超时) 返回 Err, 不会留下半开的会话。
    pub fn new(name: String, cwd: PathBuf) -> Result<Self, String> {
        Self::spawn(name, cwd, None)
    }

    /// 以指定 pi 二进制启动会话 (生产走 `None` = PATH 解析; 测试注入 stub)。
    ///
    /// 输入: name — sidebar 显示名; cwd — 工作目录; pi_bin — pi 可执行文件
    ///       (None = resolve_binary())
    /// 输出: 成功返回就绪会话 (Starting 状态, 网格为回退尺寸);
    ///       失败 (pi 缺失/探活超时) 返回 Err, 不留下半开会话
    fn spawn(name: String, cwd: PathBuf, pi_bin: Option<String>) -> Result<Self, String> {
        let config = EmbedConfig {
            pi_bin, // None: resolve_binary(): PI_BIN 覆盖或 PATH 里的 pi
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
            grid_size: (COLS, ROWS),
            status: SessionStatus::Starting,
        })
    }

    /// 测试专用: 以指定 pi 二进制 (stub) 构造会话。
    #[cfg(test)]
    pub(crate) fn new_with_pi_bin(
        name: String,
        cwd: PathBuf,
        pi_bin: Option<String>,
    ) -> Result<Self, String> {
        Self::spawn(name, cwd, pi_bin)
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

    fn send_wheel(&mut self, lines: i16) {
        self.embed.inject_wheel(lines);
    }

    fn send_click(&mut self, x: u16, y: u16) {
        self.embed.inject_click(x, y);
    }

    fn shutdown(&mut self) {
        // 退出事件循环并 join (线程侧杀 pi 子进程, spec: Manual session close)
        self.embed.shutdown();
        self.status = SessionStatus::Exited;
    }

    fn resize(&mut self, cols: u16, rows: u16) -> bool {
        // Exited: embed 线程已死且最后画面必须保留 (spec: Grid resize
        // propagation — exited 会话不 reflow)
        if self.status == SessionStatus::Exited {
            return false;
        }
        let (cols, rows) = (cols.max(1), rows.max(1));
        // 同尺寸去重: 拖动窗口时每个 distinct size 只传播一次
        if self.grid_size == (cols, rows) {
            return false;
        }
        self.grid_size = (cols, rows);
        // 两半同步: embed TUI 下一帧 autoresize 重绘, vt100 屏幕立即 reflow
        self.embed.resize(cols, rows);
        if let (Some(r), Some(c)) = (NonZeroU16::new(rows), NonZeroU16::new(cols)) {
            self.parser.set_size(r, c);
        }
        true
    }

    fn status(&self) -> SessionStatus {
        self.status
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write as _;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// 假 pi: 记录 cwd 并对每个 JSON 请求回 success
    /// (与 tests/embed_layout.rs 同款, 探活/事件循环可跑通)。
    fn write_stub(dir: &std::path::Path) -> PathBuf {
        let pwd_file = dir.join("pwd.txt");
        let script = format!(
            "#!/bin/sh\npwd > '{}'\nwhile IFS= read -r line; do\n  id=$(printf '%s' \"$line\" | sed -n 's/.*\"id\":\"\\([^\"]*\\)\".*/\\1/p')\n  printf '{{\"type\":\"response\",\"id\":\"%s\",\"success\":true,\"data\":{{}}}}\\n' \"$id\"\ndone\n",
            pwd_file.display()
        );
        let path = dir.join("fake-pi.sh");
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(script.as_bytes()).unwrap();
        drop(f);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    /// 把整个屏幕 dump 成带分隔符的字符串, 用于逐字节比较。
    fn dump(screen: &vt100::Screen) -> String {
        let (rows, cols) = screen.size();
        let mut out = String::new();
        for r in 0..rows.get() {
            for c in 0..cols.get() {
                out.push_str(screen.cell(r, c).map(|c| c.contents()).unwrap_or("\u{0}"));
                out.push('\u{1}');
            }
            out.push('\n');
        }
        out
    }

    /// spec: Adaptive terminal grid sizing / Grid resize propagation。
    /// 2.1: resize 同步两半 + 去重 + Exited 屏幕逐字节保留;
    /// 2.2: 首帧测量前会话处于回退尺寸。
    #[test]
    fn resize_updates_dedupes_and_preserves_exited_screen() {
        let unique = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("pi-shell-resize-{unique}-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let stub = write_stub(&dir);

        let mut shell = PiAgentShell::new_with_pi_bin(
            "pi-t".into(),
            dir.clone(),
            Some(stub.to_str().unwrap().to_string()),
        )
        .expect("stub embed spawn");

        // 2.2: 首帧测量前 = 回退常量 (网格与 parser 初始一致)
        assert_eq!(shell.grid_size, (COLS, ROWS));
        assert_eq!(
            shell.screen().size(),
            (NonZeroU16::new(ROWS).unwrap(), NonZeroU16::new(COLS).unwrap()),
            "新会话首帧前应回退尺寸"
        );

        // 2.1: resize 传播到 vt100 屏幕 (embed 半边经共享槽+Resize 事件,
        // 由 embed.resize 内部完成) 并记录尺寸
        assert!(shell.resize(80, 24), "尺寸变化必须传播");
        assert_eq!(shell.grid_size, (80, 24));
        assert_eq!(shell.screen().size().0.get(), 24, "vt100 行数已 reflow");
        assert_eq!(shell.screen().size().1.get(), 80, "vt100 列数已 reflow");

        // 去重: 同尺寸不再传播 (spec: Unchanged layout propagates nothing)
        assert!(!shell.resize(80, 24), "同尺寸必须去重");

        // Exited: 不传播, 屏幕逐字节保留 (spec: Exited session screen preserved)
        shell.shutdown();
        assert_eq!(shell.status(), SessionStatus::Exited);
        let before = dump(shell.screen());
        assert!(!shell.resize(60, 20), "Exited 会话不得 resize");
        assert_eq!(shell.grid_size, (80, 24), "Exited 不得更新尺寸状态");
        assert_eq!(dump(shell.screen()), before, "Exited 屏幕必须逐字节保留");

        let _ = fs::remove_dir_all(&dir);
    }
}
