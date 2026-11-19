//! 服务侧会话句柄抽象（design D2 控制面）：`AgentService` 只认本 trait，
//! 具体 embed 实现（`PiAgentShell`）与测试桩可互换。

use crate::contract::SessionId;
use pi_shell_rs::app::theme::TuiTheme;
use std::any::Any;
use std::path::PathBuf;
use std::sync::Arc;

/// 服务持有的会话 IO 面（要求 `Send + Sync`：服务整体进
/// `Service: Send + Sync + 'static`；spike task 1.1 已证 embed 句柄满足）。
pub trait SessionIo: Send + Sync + 'static {
    /// 取走本 tick 新产出的渲染字节（ANSI；**所有权移动**，空 = 无新帧）。
    fn drain(&mut self) -> Vec<u8>;
    /// io 是否仍存活（embed 事件循环线程；false → `Exited` 观测）。
    fn alive(&self) -> bool;

    // 输入注入（转发给 embed 的 UiEvent 通道）
    fn send_key(&mut self, key: crossterm::event::KeyEvent);
    fn send_text(&mut self, text: &str);
    fn send_wheel(&mut self, lines: i16);
    fn send_click(&mut self, x: u16, y: u16);
    /// 尺寸传播（实现侧负责去重语义之外的真实传播；去重在服务侧缓存）。
    fn resize(&mut self, cols: u16, rows: u16) -> bool;
    /// 注入宿主配色。
    fn set_theme(&mut self, theme: &TuiTheme);
    /// 关闭（终止 embed 线程 + 杀 pi 子进程；幂等）。
    fn shutdown(&mut self);

    /// 测试检视口（桩断言用；生产实现返回 self 即可）。
    fn as_any(&self) -> &dyn Any;
}

/// 会话工厂：`create` handler 用它产出新会话（生产 = embed spawn；
/// 测试 = 桩 + 捕获闭包）。`Err(String)` = 创建失败（spec delta 场景 3）。
pub type SessionFactory =
    Arc<dyn Fn(String, PathBuf, TuiTheme) -> Result<Box<dyn SessionIo>, String> + Send + Sync>;

/// 会话注册表条目（服务侧全部可变状态，driver 单线程顺序访问无锁）。
pub struct SessionHandle {
    pub id: SessionId,
    pub name: String,
    pub root: PathBuf,
    pub status: crate::contract::SessionStatus,
    /// 最近传播的 (cols, rows)——服务侧去重缓存（design D3 resize 行）。
    pub size: Option<(u16, u16)>,
    pub io: Box<dyn SessionIo>,
}
