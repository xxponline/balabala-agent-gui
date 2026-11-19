//! `AgentService`：进程内会话薄服务（design D1/D2/D3）。
//!
//! task 1.2 = 空骨架：身份 + 出站能力就位、`start`/`handle` 可过；
//! 注册表与各 handler 随 task 2.x 落地。

use service_core::{Envelope, InstanceId, Service, ServiceCtx, ServiceFuture};

/// 服务实例状态（空注册表骨架；`SessionId -> SessionIo` 随 task 2.1 进来）。
pub struct AgentService {
    ctx: ServiceCtx,
}

impl AgentService {
    pub fn new(ctx: ServiceCtx) -> Self {
        Self { ctx }
    }

    /// 本服务实例 id（观测/测试用）。
    pub fn instance_id(&self) -> InstanceId {
        self.ctx.instance
    }
}

impl Service for AgentService {
    fn start(&mut self) -> Result<(), String> {
        // 骨架: 无注册表可建、无 drain 可跑 —— 成功即 available。
        Ok(())
    }

    fn handle(&mut self, _env: Envelope) -> ServiceFuture<'_> {
        // task 2.2-2.5 接入控制/输入/观察 handler。
        Box::pin(std::future::ready(()))
    }
}

/// 内部服务工厂（`ServiceOrigin::Internal`，task 1.3 由 GUI
/// `register_internal("agent", agent_service_factory)` 注册装载）。
pub fn agent_service_factory(ctx: ServiceCtx) -> Box<dyn Service> {
    Box::new(AgentService::new(ctx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::test_ctx;

    /// task 1.2 验证: 空注册表 start 返回 Ok。
    #[test]
    fn start_returns_ok_on_empty_registry() {
        let mut svc = AgentService::new(test_ctx(InstanceId(7)));
        assert!(svc.start().is_ok(), "空骨架 start 必须 Ok");
    }

    #[test]
    fn factory_yields_startable_service() {
        let mut boxed: Box<dyn Service> = agent_service_factory(test_ctx(InstanceId(9)));
        assert!(boxed.start().is_ok(), "工厂出厂即可 start");
    }
}
