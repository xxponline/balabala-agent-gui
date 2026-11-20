//! 统一信封与类型化 payload（design D5）。
//!
//! - 默认 payload = `Typed(Arc<dyn Any + Send + Sync>)`：共享指针透传，零序列化。
//! - 序列化封装（JSON/bincode）**不是** `Payload` 的分支——它本身就是一个
//!   Message 类型（如 `Json(String)`），应用层自取所需。

use crate::error::RouteError;
use crate::route::{Address, Corr, Route};
use std::any::Any;
use std::fmt;
use std::sync::Arc;

/// 类型化 payload：Router 只搬 `Arc`、不解释内容（D2：信封对 payload 不解释）。
#[derive(Clone)]
pub enum Payload {
    /// 默认：共享 Message，零序列化透传。
    Typed(Arc<dyn Any + Send + Sync>),
}

impl Payload {
    pub fn new<T: Any + Send + Sync>(value: T) -> Self {
        Payload::Typed(Arc::new(value))
    }

    pub fn from_arc(arc: Arc<dyn Any + Send + Sync>) -> Self {
        Payload::Typed(arc)
    }

    /// downcast 候选：类型对不上返回 `None`（接收侧按 `RouteError::TypeMismatch`
    /// 拒收落死信，D5——这是接收方契约 bug，不是路由错误）。
    pub fn downcast<T: Any + Send + Sync>(&self) -> Option<&T> {
        match self {
            Payload::Typed(arc) => arc.as_ref().downcast_ref::<T>(),
        }
    }
}

// 注: 不提供 `impl<T: Any + Send + Sync> From<T> for Payload` ——
// Payload 自身满足该 bound, 会与核心 blanket `impl<T> From<T> for T` 冲突
// (E0119)。取而代之用 `Payload::new(value)` 显式构造。

impl fmt::Debug for Payload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Payload::Typed")
    }
}

/// 统一信封：全部消息（宿主 actor、GUI、插件 actor）的唯一载体（D3）。
#[derive(Clone, Debug)]
pub struct Envelope {
    /// 目标：两级寻址。
    pub to: Route,
    /// 发送方：**仅实例/actor 地址**（类型层保证，D5）；纯观测用。
    pub from: Address,
    /// ask 关联 id：回复凭它销账；`None` = tell。
    pub corr: Option<Corr>,
    /// 手写观测标签（日志/追踪用，**不依赖 TypeId**，D5/D12）。
    pub kind: &'static str,
    /// 类型化 payload。
    pub payload: Payload,
}

impl Envelope {
    /// 构造信封（`corr = None`，即 tell 形态）。
    pub fn new(to: Route, from: Address, kind: &'static str, payload: Payload) -> Self {
        Envelope {
            to,
            from,
            corr: None,
            kind,
            payload,
        }
    }

    /// 挂关联 id（ask 形态）。
    pub fn with_corr(mut self, corr: Corr) -> Self {
        self.corr = Some(corr);
        self
    }

    /// 按约定类型取 payload；对不上 = `TypeMismatch`（D5）。
    pub fn downcast<T: Any + Send + Sync>(&self) -> Result<&T, RouteError> {
        self.payload.downcast::<T>().ok_or_else(|| RouteError::TypeMismatch {
            expected: std::any::type_name::<T>(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::route::{InstanceId, ServiceName};

    #[derive(Debug, PartialEq, Eq)]
    struct Ping(u64);
    #[derive(Debug, PartialEq, Eq)]
    struct Pong(u64);

    fn addr() -> Address {
        Address::instance(InstanceId(1))
    }

    #[test]
    fn typed_payload_passes_without_serialization() {
        let env = Envelope::new(
            Route::Service(ServiceName::from("demo")),
            addr(),
            "ping",
            Payload::new(Ping(42)),
        );
        let ping: &Ping = env.downcast().expect("Typed payload 直取");
        assert_eq!(*ping, Ping(42));
        // Arc 语义: 两次 downcast 指向同一值, 无拷贝
        let again: &Ping = env.downcast().unwrap();
        assert_eq!(again.0, 42);
        assert_eq!(env.kind, "ping");
        assert!(env.corr.is_none(), "tell 形态 corr = None");
    }

    #[test]
    fn downcast_mismatch_is_type_error() {
        let env = Envelope::new(
            Route::Instance(InstanceId(2)),
            addr(),
            "pong",
            Payload::new(Pong(1)),
        );
        let err = env.downcast::<Ping>().expect_err("类型对不上应报 TypeMismatch");
        match err {
            RouteError::TypeMismatch { expected } => {
                assert!(expected.contains("Ping"), "expected 指名期待类型, got {expected}");
            }
            other => panic!("错误变体不对: {other:?}"),
        }
    }

    #[test]
    fn with_corr_marks_ask() {
        let env = Envelope::new(Route::Instance(InstanceId(3)), addr(), "q", Payload::new(Ping(0)))
            .with_corr(Corr(9));
        assert_eq!(env.corr, Some(Corr(9)));
    }

    #[test]
    fn payload_from_arc_and_debug() {
        let p = Payload::from_arc(Arc::new(Pong(5)));
        assert_eq!(p.downcast::<Pong>(), Some(&Pong(5)));
        assert!(p.downcast::<Ping>().is_none());
        assert_eq!(format!("{p:?}"), "Payload::Typed");
    }

    #[test]
    fn envelope_is_clone_debug() {
        let env = Envelope::new(Route::Instance(InstanceId(4)), addr(), "k", Payload::new(Ping(1)));
        let env2 = env.clone();
        assert_eq!(env2.kind, "k");
        assert!(format!("{env:?}").contains("Envelope"));
    }
}
