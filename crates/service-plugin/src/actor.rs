//! kameo actor 脚手架（design D1：插件内的服务本体是 kameo actor）。
//!
//! ```ignore
//! // 插件侧典型用法 (见 crates/example-plugin):
//! let service = service_plugin::actor_service(ctx, (), |actor, ctx| {
//!     Box::new(MyIngress { actor: actor.clone(), ctx: ctx.clone() })
//! });
//! ```
//!
//! - [`ActorService::start`] 用 `A::spawn` **裸 spawn**：不建 supervision 链
//!   ⇒ **kameo 自动 restart 结构上不存在**，crash 由宿主 driver 的 JoinErr
//!   上报落 `error`（supervisor 唯一恢复权威，D7）；
//! - ingress 是插件作者的分发实现：`Envelope` → downcast 类型化消息 →
//!   `actor.tell/ask`（与宿主共用同一份契约类型，D1 边界立场）。

use kameo::actor::{Actor, ActorRef, Spawn};
use service_core::{Envelope, Service, ServiceCtx, ServiceFuture};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

/// 从 `ServiceCtx::runtime_token` 取宿主运行时句柄（同构建 TypeId 前提与
/// D12 一致；core 不依赖 tokio，故走不透明载体）。
fn runtime_handle(ctx: &ServiceCtx) -> Option<tokio::runtime::Handle> {
    let ptr = ctx.runtime_token.handle_ptr();
    if ptr.is_null() {
        return None;
    }
    // SAFETY: 宿主保证指针指向存活的 `tokio::runtime::Handle`；两侧同 rustc /
    // 同 tokio 版本 / 同 target ⇒ 布局一致（D12 路线甲的"一起编译"前提）。
    // 不走 TypeId downcast：tokio 的 proc-macro 依赖链 metadata 随 cargo 解析
    // 范围变化，跨 dylib TypeId 不可靠（已踩：workspace vs -p 构建分叉）。
    let handle: &tokio::runtime::Handle = unsafe { &*(ptr as *const tokio::runtime::Handle) };
    Some(handle.clone())
}

/// 每次 poll 前 `enter` 宿主运行时上下文的组合子：跨 dylib 时插件的
/// `tokio::spawn`/timer 读的是插件自己那份 TLS——必须由本组合子逐 poll
/// 注入（kameo 内部的 links 通知等会调 tokio::spawn，已踩过）。
struct EnterFut<'a> {
    handle: Option<tokio::runtime::Handle>,
    fut: Pin<Box<dyn Future<Output = ()> + Send + 'a>>,
}

impl<'a> Future for EnterFut<'a> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = self.get_mut();
        let _guard = this.handle.as_ref().map(|h| h.enter());
        this.fut.as_mut().poll(cx)
    }
}

/// ingress 分发 future（owner：信封按值进入，不跨函数悬垂）。
pub type IngressFuture<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

/// 插件作者实现：把信封翻译成 actor 消息（downcast → tell/ask）。
/// downcast 失败时调 `ctx.messaging.reject(&env)`（落死信 `TypeMismatch`，spec R1）。
pub trait Ingress: Send + Sync + 'static {
    fn dispatch(&self, env: Envelope) -> IngressFuture<'_>;
}

/// 由 [`actor_service`] 构造的宿主驱动服务（每实例一个）。
pub struct ActorService<A: Actor> {
    ctx: ServiceCtx,
    args: Option<A::Args>,
    build: Option<Box<dyn FnOnce(&ActorRef<A>, &ServiceCtx) -> Box<dyn Ingress> + Send + Sync>>,
    ingress: Option<Box<dyn Ingress>>,
    /// 宿主运行时句柄（逐 poll enter 用；start 时从 token 取出）。
    runtime: Option<tokio::runtime::Handle>,
}

/// 包一个 kameo actor 为宿主驱动的 `Service`：
/// `build` 在 actor spawn 后被调用（runtime 上下文内），返回该实例的 ingress。
pub fn actor_service<A: Actor>(
    ctx: ServiceCtx,
    args: A::Args,
    build: impl FnOnce(&ActorRef<A>, &ServiceCtx) -> Box<dyn Ingress> + Send + Sync + 'static,
) -> ActorService<A> {
    ActorService {
        ctx,
        args: Some(args),
        build: Some(Box::new(build)),
        ingress: None,
        runtime: None,
    }
}

impl<A> Service for ActorService<A>
where
    A: Actor + Sync + 'static,
    A::Args: Send + Sync + 'static,
{
    fn start(&mut self) -> Result<(), String> {
        let args = self.args.take().ok_or_else(|| "start() called twice".to_string())?;
        let build = self.build.take().ok_or_else(|| "start() called twice".to_string())?;
        // enter 宿主运行时上下文 (同步段: 插件侧 kameo/tokio 依赖此 TLS)
        let runtime = runtime_handle(&self.ctx);
        let _guard = runtime.as_ref().map(|h| h.enter());
        self.runtime = runtime;
        // 裸 prepare: 无链接/无 supervision ⇒ 无自动 restart (D7)。
        // **actor 生命周期交给宿主 runtime 驱动** (Messaging::spawn_task):
        // 跨 dylib 时 tokio ambient 上下文是插件自己的 TLS, 看不到宿主
        // runtime —— 插件侧绝不自调裸 tokio::spawn (D12 边界发现)。
        let prepared = A::prepare();
        let actor_ref = prepared.actor_ref().clone();
        let run = prepared.run(args);
        let run = EnterFut {
            handle: self.runtime.clone(),
            fut: Box::pin(async move {
                match run.await {
                    Ok(_) => {} // 正常停止
                    Err(_panic) => {
                        // kameo 捕到 actor panic → 转成宿主可观察的 crash (D7)
                        panic!("kameo actor panicked");
                    }
                }
            }),
        };
        self.ctx.messaging.spawn_task(Box::pin(run));
        self.ingress = Some(build(&actor_ref, &self.ctx));
        Ok(())
    }

    fn handle(&mut self, env: Envelope) -> ServiceFuture<'_> {
        match &self.ingress {
            Some(ingress) => {
                let fut = ingress.dispatch(env);
                // 逐 poll enter: ingress 内部的 ask/tell (kameo) 可能触发
                // tokio::spawn/timer (插件侧 TLS)
                Box::pin(EnterFut {
                    handle: self.runtime.clone(),
                    fut: Box::pin(fut),
                })
            }
            // driver 顺序保证: start 成功后才收件; 防御分支
            None => Box::pin(std::future::ready(())),
        }
    }
}

/// D12 **路线乙预留**（v1 不启用）：downcast 永远在宿主侧——DLL 的入口是
/// 宿主侧 typed extern fn，编好后以 `&Msg` 直递进插件，插件侧永不碰
/// `Any`/`TypeId`。甲路线（指纹 + round-trip 测试）失败即切换，信封与
/// Router 不动。未来形状（Bridge change 落实）：
///
/// ```text
/// #[no_mangle] unsafe extern "C-unwind" fn on_ping(
///     ctx: *mut std::ffi::c_void,   // 宿主侧 typed 闭包上下文
///     msg: &contracts::demo::Ping,  // 类型直递, 零 downcast
/// )
/// ```
///
/// 本 trait 是插件侧对应能力的挂点标记。
pub trait TypedIngress<M: Send + Sync + 'static>: Send + Sync {
    fn on_message(&self, msg: &M);
}
