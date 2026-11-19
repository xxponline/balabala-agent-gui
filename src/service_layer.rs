//! GUI 侧服务接入（service-management-framework task 6.1）。
//!
//! **GUI 就是总线上的拉模式参与者**（design D9/D10）：
//! - 启动时 bootstrap：扫描 `plugins/` 建 Catalog、装载核心假服务、
//!   注册 GUI 自己的 mailbox（`Listener`）、订阅状态翻转（订阅 = 路由规则）；
//! - 泵：`pump()` 并入现有 50ms drain 循环（`poll_sessions`）——listener
//!   poll 同步、GUI 主线程执行，**无回调、无跨线程合同**；
//! - 用户触发的操作一律走 ask（`ask_core`）——错误可回显；tell 只用于
//!   fire-and-forget（D6 约定）。
//!
//! 不含服务状态页 UX（6.2 验收边界：留给后续 GUI spec change），不迁移
//! 任何现有模块。

use service_manager::{
    InstanceId, Listener, Payload, Route, Service, ServiceCtx, ServiceFuture, ServiceManager,
    ServiceState, StateChange, KIND_ROUTE_ERROR, KIND_STATE_CHANGE,
};
use std::future::Future;
use std::path::Path;

/// GUI → core.fake 的请求（**归调用方**，D2 归属规则演示）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CorePing(pub u64);

/// core.fake 的应答。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CorePong(pub u64);

/// 核心假服务（framework 自带的最小可服务载体；真模块迁移属后续 change）。
struct CoreFake {
    ctx: ServiceCtx,
}

impl Service for CoreFake {
    fn start(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn handle(&mut self, env: service_manager::Envelope) -> ServiceFuture<'_> {
        if env.corr.is_none() {
            // tell 语义: 无回复; 未知消息拒收落死信 (spec R1)
            let _ = self.ctx.messaging.reject(&env);
            return Box::pin(std::future::ready(()));
        }
        match env.downcast::<CorePing>() {
            Ok(ping) => {
                let _ = self
                    .ctx
                    .messaging
                    .reply(&env, "core.pong", Payload::new(CorePong(ping.0)));
            }
            Err(_) => {
                let _ = self.ctx.messaging.reject(&env);
            }
        }
        Box::pin(std::future::ready(()))
    }
}

fn core_fake_factory(ctx: ServiceCtx) -> Box<dyn Service> {
    Box::new(CoreFake { ctx })
}

/// GUI 参与者的服务层状态（订阅 + 本地快照，D10 投影所有权在 GUI 侧）。
pub struct ServiceLayer {
    /// 总线门面（宿主 API，零 tokio/kameo 类型，D9）。
    pub manager: ServiceManager,
    /// GUI 自己的 mailbox（拉模式 actor 句柄）。
    pub listener: Listener,
    /// 核心假服务实例 id。
    pub core_id: InstanceId,
    /// agent 会话薄服务实例 id（change agent-service task 1.3；控制/帧路由目标）。
    #[allow(dead_code)] // task 3.1 watch/控制路由接线后读取
    pub agent_id: InstanceId,
    /// 经泵消费到的事件数（测试观测面：证明 Listener 接入 gpui 泵）。
    pub events_seen: usize,
    /// 最近一次状态翻转（D10 本地快照；状态页 UX 属后续 change）。
    pub last_state: Option<ServiceState>,
}

impl ServiceLayer {
    /// 启动接线：扫描 `scan_dir`（= 设计中的 `plugins/` 落点）建 Catalog，
    /// 注册并装载核心假服务，注册 GUI mailbox、订阅其状态翻转。
    pub fn bootstrap(scan_dir: &Path) -> Result<Self, String> {
        let manager = ServiceManager::new(scan_dir).map_err(|e| format!("service layer: {e}"))?;
        manager
            .register_internal("core.fake", core_fake_factory)
            .map_err(|e| format!("register core.fake: {e}"))?;
        let core_id = manager
            .load(&"core.fake".into())
            .map_err(|e| format!("load core.fake: {e}"))?;
        // change agent-service task 1.3: 进程内会话薄服务 (design D1)
        manager
            .register_internal("agent", agent_service::agent_service_factory)
            .map_err(|e| format!("register agent: {e}"))?;
        let agent_id = manager
            .load(&"agent".into())
            .map_err(|e| format!("load agent: {e}"))?;
        // GUI = 普通参与者: 自己的 mailbox + 订阅 (= 往路由表加规则, D7/D9)
        let listener = manager.new_mailbox(64);
        manager.subscribe_state(core_id, &listener);
        manager.subscribe_state(agent_id, &listener);
        Ok(ServiceLayer {
            manager,
            listener,
            core_id,
            agent_id,
            events_seen: 0,
            last_state: None,
        })
    }

    /// 一次消息循环迭代：poll mailbox、逐条消费（状态事件更新本地快照，
    /// route_error 可见化预留）。返回是否有新件——调用方据此 `notify`。
    pub fn pump(&mut self) -> bool {
        if !self.listener.poll() {
            return false;
        }
        let mut changed = false;
        for env in self.listener.drain() {
            changed = true;
            match env.kind {
                KIND_STATE_CHANGE => {
                    if let Ok(change) = env.downcast::<StateChange>() {
                        self.last_state = Some(change.to);
                    }
                    self.events_seen += 1;
                }
                KIND_ROUTE_ERROR => {
                    // ask 失败/超时对 poll 调用方可见 (D6); UX 归后续 spec
                    self.events_seen += 1;
                }
                _ => {
                    self.events_seen += 1;
                }
            }
        }
        changed
    }

    /// GUI 用户操作的 ask 通道（约定：**一律 ask，不 tell**，D6）。
    /// 返回 executor-agnostic future，gpui executor 可直接 await。
    /// （6.2 验收边界：状态页/按钮 UX 归后续 GUI spec change，本 change 只
    /// 备好 ask 通道并测通；届时用户操作从 UI 调此方法。）
    #[allow(dead_code)] // UI 接入属后续 change — 通道已测通, 非死码
    pub fn ask_core(
        &self,
        seq: u64,
    ) -> impl Future<Output = Result<CorePong, service_manager::RouteError>> + Send + 'static {
        let env = service_manager::Envelope::new(
            Route::Instance(self.core_id),
            self.listener.address().clone(),
            "core.ping",
            Payload::new(CorePing(seq)),
        );
        let fut = self.manager.call(env); // 'static,急切分发 (D9)
        async move {
            let payload = fut.await?;
            payload
                .downcast::<CorePong>()
                .copied()
                .ok_or(service_manager::RouteError::TypeMismatch {
                    expected: "service_layer::CorePong",
                })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn wait_until(mut cond: impl FnMut() -> bool, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if cond() {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn temp_scan(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "smf-gui-{tag}-{}-{:04x}",
            std::process::id(),
            (Instant::now().elapsed().as_nanos() ^ 0x5155) as u16
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn bootstrap_loads_core_fake_and_subscribes() {
        let dir = temp_scan("boot");
        let mut layer = ServiceLayer::bootstrap(&dir).expect("bootstrap");
        assert!(layer.manager.has_service(&"core.fake".into()), "Catalog 登记");
        assert_eq!(layer.manager.instance_count(), 2, "core.fake + agent 均已装载");

        // init 完成 (loading → available 事件应达订阅者)
        assert!(
            wait_until(
                || {
                    layer.pump();
                    layer.last_state == Some(ServiceState::Available)
                },
                Duration::from_secs(3)
            ),
            "状态翻转经订阅路由到达 GUI mailbox: {:?}",
            layer.last_state
        );
        assert!(layer.events_seen >= 1);

        // ask (GUI 用户操作约定): typed 一问一答
        let pong = futures::executor::block_on(layer.ask_core(41)).expect("ask ok");
        assert_eq!(pong, CorePong(41));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// change agent-service task 1.3: agent 薄服务注册/装载 + 状态事件到 pump。
    #[test]
    fn agent_service_loads_and_reports_state() {
        let dir = temp_scan("agent-boot");
        let mut layer = ServiceLayer::bootstrap(&dir).expect("bootstrap");
        assert!(layer.manager.has_service(&"agent".into()), "Catalog 登记 agent");
        assert_eq!(layer.manager.instance_count(), 2, "core.fake + agent");
        assert_ne!(layer.agent_id, layer.core_id, "实例 id 独立分配");

        // loading → available 翻转经订阅路由到达 GUI mailbox
        assert!(
            wait_until(
                || {
                    layer.pump();
                    layer.last_state == Some(ServiceState::Available)
                },
                Duration::from_secs(3)
            ),
            "agent 状态翻转到 Available: {:?}",
            layer.last_state
        );
        assert!(layer.events_seen >= 1, "状态事件被 pump 消费");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn disable_event_visible_to_gui_pump() {
        let dir = temp_scan("disable");
        let mut layer = ServiceLayer::bootstrap(&dir).expect("bootstrap");
        assert!(wait_until(
            || {
                layer.pump();
                layer.last_state == Some(ServiceState::Available)
            },
            Duration::from_secs(3)
        ));
        let seen_before = layer.events_seen;

        layer.manager.disable(layer.core_id).expect("disable");
        assert!(wait_until(
            || {
                layer.pump();
                layer.last_state == Some(ServiceState::Offline)
            },
            Duration::from_secs(3)
        ));
        assert!(layer.events_seen > seen_before, "新事件被消费");
        std::fs::remove_dir_all(&dir).ok();
    }
}
