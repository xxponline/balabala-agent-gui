//! 示例插件（task 5.1/5.2）：D12 边界载体 + 集成测试装载对象。
//!
//! 结构 = 插件侧标准形态（`service-plugin` 脚手架）：
//! - 导出构建指纹（build.rs 生成，`SERVICE_CONTRACT_FINGERPRINT`）；
//! - 导出工厂 `service_create`（`ServiceCtx` 进、`Box<dyn Service>` 出，
//!   与内部服务**同一契约**，D1——仅"符号来源"不同）；
//! - 服务本体 = kameo actor（**裸 spawn，无自动 restart**，D7/D1）+ ingress
//!   分发：`Envelope` → downcast `contracts::demo::Ping` → actor ask →
//!   `Pong` 经总线回复（回复带 corr 销账，D5/D6）。

use contracts::demo::{Ping, Pong};
use kameo::actor::{Actor, ActorRef};
use kameo::message::{Context, Message};
use kameo::error::Infallible;
use service_core::{Envelope, Payload, Service, ServiceCtx};
use service_plugin::{actor_service, Ingress, IngressFuture};

// 构建指纹导出符号 (task 3.2; Loader 装载前比对, 不匹配显式拒载)
include!(concat!(env!("OUT_DIR"), "/service_fingerprint.rs"));

/// demo actor：收 `Ping` 回 `Pong`（回显值）。
struct EchoActor;

impl Actor for EchoActor {
    type Args = ();
    type Error = Infallible;

    async fn on_start(_args: Self::Args, _actor: ActorRef<Self>) -> Result<Self, Self::Error> {
        Ok(EchoActor)
    }
}

// kameo Reply 只覆盖内置类型 (契约 crate 保持零依赖, 不 derive kameo::Reply);
// actor 内部跳用 u64, 契约类型 Pong 仍在信封边界 (messaging.reply) 往返。
impl Message<Ping> for EchoActor {
    type Reply = u64;

    async fn handle(&mut self, msg: Ping, _ctx: &mut Context<Self, Self::Reply>) -> u64 {
        msg.0
    }
}

/// ingress：信封 → 类型化消息 → kameo ask → 总线回复。
struct DemoIngress {
    actor: ActorRef<EchoActor>,
    ctx: ServiceCtx,
}

impl Ingress for DemoIngress {
    fn dispatch(&self, env: Envelope) -> IngressFuture<'_> {
        let actor = self.actor.clone();
        let ctx = self.ctx.clone();
        Box::pin(async move {
            // downcast 失败 → 拒收落死信 (spec R1)
            let ping = match env.downcast::<Ping>() {
                Ok(p) => p.clone(),
                Err(_) => {
                    let _ = ctx.messaging.reject(&env);
                    return;
                }
            };
            let pong = match actor.ask(ping).await {
                Ok(value) => Pong(value),
                Err(_) => return, // actor 异常消亡 (不自动拉起语义下不会自愈; v1 观测缺省)
            };
            if env.corr.is_some() {
                let _ = ctx
                    .messaging
                    .reply(&env, "demo.pong", Payload::new(pong));
            }
        })
    }
}

/// 插件入口（与内部服务同一 `DllServiceFactory` 契约，D1/D8）。
/// `C-unwind`：工厂 panic 穿回宿主被 `LoadError::FactoryPanic` 接住。
#[unsafe(no_mangle)]
#[allow(improper_ctypes_definitions)]
pub unsafe extern "C-unwind" fn service_create(ctx: ServiceCtx) -> Box<dyn Service> {
    Box::new(actor_service(ctx, (), |actor, ctx| {
        Box::new(DemoIngress {
            actor: actor.clone(),
            ctx: ctx.clone(),
        })
    }))
}
