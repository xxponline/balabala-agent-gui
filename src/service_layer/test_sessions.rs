//! 测试会话基础设施（change agent-service task 3.2/3.3；仅 `cfg(test)`）。
//!
//! 测试构建下 `bootstrap` 注册的工厂产出**可检视**测试会话：登记表按
//! project root 隔离（各测试 root 唯一 → 并行零互扰），测试持
//! `Arc<Mutex<TestSession>>` 直读转发记录/注入帧字节。

#![allow(dead_code)] // 各测试按需取用; 整体为测试地基

use agent_service::session::{SessionFactory, SessionIo};
use pi_shell_rs::app::theme::TuiTheme;
use std::any::Any;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

/// 可检视的测试会话状态（经 `SharedIo` 与服务侧共享）。
#[derive(Default)]
pub struct TestSession {
    pub alive: bool,
    /// 下次 drain 返回并清空的帧字节。
    pub pending_bytes: Vec<u8>,
    pub keys: Vec<crossterm::event::KeyEvent>,
    pub texts: Vec<String>,
    pub wheels: Vec<i16>,
    pub clicks: Vec<(u16, u16)>,
    pub resized: Vec<(u16, u16)>,
    pub themes: Vec<TuiTheme>,
    pub shutdowns: u32,
}

impl TestSession {
    pub fn running() -> Self {
        Self {
            alive: true,
            ..Self::default()
        }
    }
}

/// 共享句柄：服务侧 `SessionIo` 与测试断言共用同一份状态。
pub type SharedSession = Arc<Mutex<TestSession>>;

fn registry() -> &'static Mutex<HashMap<PathBuf, Vec<SharedSession>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<PathBuf, Vec<SharedSession>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 某 root 下已登记的全部测试会话（按创建序）。
pub fn sessions_of(root: &std::path::Path) -> Vec<SharedSession> {
    registry()
        .lock()
        .expect("test session registry")
        .get(root)
        .cloned()
        .unwrap_or_default()
}

/// 包一层共享状态的 `SessionIo`（服务侧持有）。
struct SharedIo(SharedSession);

impl SessionIo for SharedIo {
    fn drain(&mut self) -> Vec<u8> {
        let mut s = self.0.lock().expect("test session");
        std::mem::take(&mut s.pending_bytes)
    }
    fn alive(&self) -> bool {
        self.0.lock().expect("test session").alive
    }
    fn send_key(&mut self, key: crossterm::event::KeyEvent) {
        self.0.lock().expect("test session").keys.push(key);
    }
    fn send_text(&mut self, text: &str) {
        self.0.lock().expect("test session").texts.push(text.into());
    }
    fn send_wheel(&mut self, lines: i16) {
        self.0.lock().expect("test session").wheels.push(lines);
    }
    fn send_click(&mut self, x: u16, y: u16) {
        self.0.lock().expect("test session").clicks.push((x, y));
    }
    fn resize(&mut self, cols: u16, rows: u16) -> bool {
        self.0.lock().expect("test session").resized.push((cols, rows));
        true
    }
    fn set_theme(&mut self, theme: &TuiTheme) {
        self.0
            .lock()
            .expect("test session")
            .themes
            .push(theme.clone());
    }
    fn shutdown(&mut self) {
        self.0.lock().expect("test session").shutdowns += 1;
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// 测试工厂：产出登记入表的共享会话（`create` 的 root = 登记键）。
///
/// **root 标记文件约定**：root 下存在 `.pi_bin`（内容 = 二进制路径）时
/// 真实 spawn 嵌入会话（stub pi e2e — input_kick 类测试用）；否则产出
/// 可检视测试会话（计数/帧脚本）。
pub fn factory() -> SessionFactory {
    Arc::new(|name, root, _theme| {
        let marker = root.join(".pi_bin");
        if marker.exists() {
            let bin = std::fs::read_to_string(&marker)
                .map_err(|e| format!(".pi_bin 读取失败: {e}"))?;
            let shell = agent_service::PiAgentShell::new_with_pi_bin(
                name,
                root,
                Some(bin.trim().to_string()),
            )?;
            return Ok(Box::new(shell) as Box<dyn SessionIo>);
        }
        let session: SharedSession = Arc::new(Mutex::new(TestSession::running()));
        registry()
            .lock()
            .expect("test session registry")
            .entry(root)
            .or_default()
            .push(Arc::clone(&session));
        Ok(Box::new(SharedIo(session)) as Box<dyn SessionIo>)
    })
}

/// 经服务创建会话并等投影镜像就位（真实 tokio 线程 → 轮询等）。
/// 返回 (SessionId, 共享状态) — 真 spawn 路径无共享状态（None）。
pub fn create_session(
    cx: &mut gpui_kit::TestAppContext,
    app: &gpui_kit::Entity<crate::app::TerminalApp>,
    root: &std::path::Path,
) -> (agent_service::contract::SessionId, Option<SharedSession>) {
    use agent_service::contract::SessionId;
    // 基线: 同 root 已有会话 — 等"新增"而非"存在" (同 root 多次 create)
    let before: Vec<SessionId> = app.read_with(cx, |app, _| {
        app.service_layer
            .sessions_view()
            .map(|v| {
                v.sessions
                    .iter()
                    .filter(|s| s.project_root == root)
                    .map(|s| s.id)
                    .collect()
            })
            .unwrap_or_default()
    });
    app.update(cx, |app, cx| app.create_agent(root, cx));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        cx.run_until_parked();
        // 手动驱动泵: 多数测试不启 50ms 更新循环 — view 镜像只能靠
        // poll_sessions 消费 mailbox (真实 tokio 线程的回执/投影事件)
        app.update(cx, |app, cx| {
            app.poll_sessions(cx);
        });
        let found: Option<SessionId> = app.read_with(cx, |app, _| -> Option<SessionId> {
            let view = app.service_layer.sessions_view()?;
            view.sessions
                .iter()
                .filter(|s| s.project_root == root && !before.contains(&s.id))
                .map(|s| s.id)
                .max()
        });
        if let Some(id) = found {
            let shared = sessions_of(root).last().cloned();
            // 镜像自举: 主动同步 active/id 清单 (部分测试不跑 50ms 泵;
            // pump 路径由 task 3.x 专项测试覆盖 — 此处仅定基准态)
            app.update(cx, |app, cx| {
                app.active = Some(id);
                if let Some(ws) = app.workspace.as_mut() {
                    if !ws.agents.contains(&id) {
                        ws.agents.push(id);
                    }
                }
                app.sync_terminal_view(cx);
            });
            return (id, shared);
        }
        assert!(
            std::time::Instant::now() < deadline,
            "create 会话超时: {root:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

pub fn inject_view(
    cx: &mut gpui_kit::TestAppContext,
    app: &gpui_kit::Entity<crate::app::TerminalApp>,
    entries: Vec<(PathBuf, String, agent_service::contract::SessionStatus)>,
    active_ix: Option<usize>,
) -> Vec<agent_service::contract::SessionId> {
    let mut ids = Vec::new();
    for (root, _name, _status) in entries {
        // 经服务创建 (TestSession 工厂): 服务投影是 sessions_view 的唯一
        // 写者 — 直写字段会被冷启动/事件投影覆盖 (实测教训)。
        let (id, _shared) = create_session(cx, app, &root);
        ids.push(id);
    }
    if let Some(ix) = active_ix {
        let target = ids[ix];
        app.update(cx, |app, _cx| {
            use agent_service::contract::{KIND_SELECT, SelectReq};
            use service_manager::{Payload, Route};
            let _ = app.service_layer.submit_control(
                Route::Service("agent".into()),
                crate::service_layer::CtrlOp::Select,
                KIND_SELECT,
                Payload::new(SelectReq(target)),
                "切换会话",
            );
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            cx.run_until_parked();
            app.update(cx, |app, cx| {
                app.poll_sessions(cx);
            });
            if app.read_with(cx, |app, _| app.active) == Some(target) {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "select 回执未镜像 active"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    ids
}

/// 轮询等待（真实 tokio 线程 + 虚拟时钟混合；每轮推进泵）。

// ---------- 计数句柄 (property_detail 测试的 `inject_session` 返回值) ----------
// 形状保持 `.get()` 读法: NumCounter(usize→i64 语义) + ClickCounter,
// 调用点 `let (keys, texts, wheels, clicks) = ...` 与断言零改动。

/// 计数维度。
pub enum Metric {
    Keys,
    Texts,
    /// 累计和 (旧行为: Cell 随 send_wheel 累加)。
    Wheels,
}

/// 数值计数句柄（读时从共享测试会话取）。
pub struct NumCounter {
    session: SharedSession,
    metric: Metric,
}

impl NumCounter {
    pub fn get(&self) -> i64 {
        let s = self.session.lock().expect("test session");
        match self.metric {
            Metric::Keys => s.keys.len() as i64,
            Metric::Texts => s.texts.len() as i64,
            Metric::Wheels => s.wheels.iter().map(|&x| i64::from(x)).sum(),
        }
    }
}

/// 点击坐标句柄 (最后一次点击; 旧行为: Cell<Option> 覆写)。
pub struct ClickCounter {
    session: SharedSession,
}

impl ClickCounter {
    pub fn get(&self) -> Option<(u16, u16)> {
        self.session.lock().expect("test session").clicks.last().copied()
    }
}

/// 从共享会话取计数句柄组 (inject_session 用)。
pub fn counters(
    session: &SharedSession,
) -> (NumCounter, NumCounter, NumCounter, ClickCounter) {
    (
        NumCounter {
            session: Arc::clone(session),
            metric: Metric::Keys,
        },
        NumCounter {
            session: Arc::clone(session),
            metric: Metric::Texts,
        },
        NumCounter {
            session: Arc::clone(session),
            metric: Metric::Wheels,
        },
        ClickCounter {
            session: Arc::clone(session),
        },
    )
}
pub fn wait_until(
    cx: &mut gpui_kit::TestAppContext,
    mut pred: impl FnMut() -> bool,
    what: &str,
) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        cx.run_until_parked();
        if pred() {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "等待超时: {what}"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}
