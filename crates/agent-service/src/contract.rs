//! 消息契约（design D3）：kind 常量与载荷类型——GUI 与 `AgentService`
//! 之间唯一的共享词汇。全部类型宿主同二进制（非 dylib 边界，不进
//! `contracts`）；payload 经 `Payload::Typed(Arc<..>)` 零拷贝透传。

use pi_shell_rs::app::theme::TuiTheme;
use std::path::PathBuf;
use std::sync::Arc;

// ---------- 身份与投影 ----------

/// 会话稳定身份（服务分配；`gpui EntityId` 消亡后的替代，design D4）。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct SessionId(pub u64);

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// 会话语义状态（与旧 GUI `SessionStatus` 同义，随 change 移入服务侧）：
/// `Starting`（已创建无输出）→ `Running`（已有输出）→ `Exited`（io 终止）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SessionStatus {
    Starting,
    Running,
    Exited,
}

/// 单会话清单项（列表/徽章/状态栏渲染源）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionInfo {
    pub id: SessionId,
    pub name: String,
    pub project_root: PathBuf,
    pub status: SessionStatus,
}

/// 全量会话投影（`sessions_changed` 事件载荷 = 快照，零回程，D10）。
#[derive(Clone, Debug)]
pub struct SessionView {
    pub sessions: Arc<[SessionInfo]>,
    pub active: Option<SessionId>,
}

// ---------- 控制（GUI → 服务，一律 ask，D6） ----------

pub struct SelectReq(pub SessionId);
pub struct CreateReq {
    pub root: PathBuf,
    // spawn 配色取服务持有的最近 `theme` 快照（`ThemeReq` 先行广播），
    // 载荷保持最小（D3 表）。
}
pub struct CloseReq(pub SessionId);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectOk;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloseOk;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreateOk(pub SessionId);

/// 控制失败载荷（spec delta: notice 点名失败操作的文案来源）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControlErr(pub String);

/// 控制回执：`Result<Ok, ControlErr>`（回复 kind = `KIND_CONTROL_REPLY`）。
pub type ControlResult<T> = Result<T, ControlErr>;

// ---------- 输入/设备（GUI → 服务，tell，静默失败，D6） ----------

pub struct InputKey {
    pub session: SessionId,
    pub key: crossterm::event::KeyEvent,
}
pub struct InputText {
    pub session: SessionId,
    pub text: String,
}
pub struct InputWheel {
    pub session: SessionId,
    pub lines: i16,
}
pub struct InputClick {
    pub session: SessionId,
    pub x: u16,
    pub y: u16,
}
pub struct ResizeReq {
    pub session: SessionId,
    pub cols: u16,
    pub rows: u16,
}
/// 主题广播（同时作为服务侧 create 时的注入快照）。
#[derive(Clone, Debug)]
pub struct ThemeReq {
    pub theme: TuiTheme,
}

// ---------- 观察/订阅（服务 → GUI） ----------

/// watch 握手载荷（订阅身份取信封 `from`，不走载荷，D3）。
pub struct WatchReq;

/// 帧全推载荷：字节所有权**移动**（`take_ansi` → Arc），传输零拷贝。
#[derive(Clone, Debug)]
pub struct FrameBytes {
    pub session: SessionId,
    pub bytes: Arc<Vec<u8>>,
}

/// 会话投影事件（事件自带快照）。
#[derive(Clone, Debug)]
pub struct SessionsChanged(pub SessionView);

// ---------- kind 常量（Envelope.kind；追踪/分发用，不依赖 TypeId，D5） ----------

pub const KIND_SELECT: &str = "select";
pub const KIND_CREATE: &str = "create";
pub const KIND_CLOSE: &str = "close";
pub const KIND_INPUT_KEY: &str = "input.key";
pub const KIND_INPUT_TEXT: &str = "input.text";
pub const KIND_INPUT_WHEEL: &str = "input.wheel";
pub const KIND_INPUT_CLICK: &str = "input.click";
pub const KIND_RESIZE: &str = "resize";
pub const KIND_THEME: &str = "theme";
pub const KIND_WATCH: &str = "watch";
pub const KIND_FRAME_BYTES: &str = "frame_bytes";
pub const KIND_SESSIONS_CHANGED: &str = "sessions_changed";
/// 控制 ask 的回执 kind（载荷 = 各操作的 `ControlResult<_>`）。
pub const KIND_CONTROL_REPLY: &str = "control.reply";
