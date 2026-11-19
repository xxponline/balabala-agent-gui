//! 路由错误词汇（design D6："错误至少含"以下变体）。

use crate::state::ServiceState;
use std::fmt;

/// 死信原因（可观测面；ask 也据此包装为 [`RouteError::DeadLetter`]）。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum DeadLetterReason {
    /// 目标 per-outlet 有界通道满：**投递失败，不是隐式缓冲**（D6）。
    ChannelFull,
    /// tell 打到非 available 目标（tell 无同步反馈，只有观测面，D6）。
    TargetNotAvailable(ServiceState),
    /// 接收方 downcast 失败（接收方契约 bug，D5）。
    TypeMismatch,
    /// 目标地址不存在。
    NoSuchRoute,
    /// 其他显式原因。
    Other(&'static str),
}

impl fmt::Display for DeadLetterReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeadLetterReason::ChannelFull => write!(f, "per-outlet channel full"),
            DeadLetterReason::TargetNotAvailable(s) => write!(f, "target not available: {s}"),
            DeadLetterReason::TypeMismatch => write!(f, "payload type mismatch"),
            DeadLetterReason::NoSuchRoute => write!(f, "no such route"),
            DeadLetterReason::Other(what) => write!(f, "{what}"),
        }
    }
}

/// 调用方 API 的错误面：`Result<Reply, RouteError>`（D6）。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RouteError {
    /// Router 持 deadline，到期销账并落死信——超时是可观测事件（D6）。
    Timeout,
    /// fail-fast：目标处于 loading/offline/error，**立即**拒绝（D6）。
    TargetNotAvailable(ServiceState),
    /// 投递失败落死信（含 ChannelFull：调用方不干等超时，D6）。
    DeadLetter(DeadLetterReason),
    /// 路由表查无此地址 / 服务无可用实例。
    NoSuchRoute,
    /// payload downcast 失败（回复或消息与约定类型对不上，D5）。
    TypeMismatch {
        /// 期待的具体类型名（`std::any::type_name`）。
        expected: &'static str,
    },
    /// `from` 收到 `Service(..)` 歧义地址（类型层由 [`crate::Address`] 拒绝，D5）。
    AmbiguousAddress,
}

impl fmt::Display for RouteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RouteError::Timeout => write!(f, "route: request timed out"),
            RouteError::TargetNotAvailable(s) => write!(f, "route: target not available ({s})"),
            RouteError::DeadLetter(reason) => write!(f, "route: dead letter ({reason})"),
            RouteError::NoSuchRoute => write!(f, "route: no such route"),
            RouteError::TypeMismatch { expected } => {
                write!(f, "route: payload type mismatch (expected {expected})")
            }
            RouteError::AmbiguousAddress => {
                write!(f, "route: ambiguous Service(..) address not allowed here")
            }
        }
    }
}

impl std::error::Error for RouteError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_names_every_variant() {
        assert_eq!(RouteError::Timeout.to_string(), "route: request timed out");
        assert_eq!(
            RouteError::TargetNotAvailable(ServiceState::Offline).to_string(),
            "route: target not available (offline)"
        );
        assert_eq!(
            RouteError::DeadLetter(DeadLetterReason::ChannelFull).to_string(),
            "route: dead letter (per-outlet channel full)"
        );
        assert_eq!(RouteError::NoSuchRoute.to_string(), "route: no such route");
        assert_eq!(
            RouteError::TypeMismatch {
                expected: "demo.Pong"
            }
            .to_string(),
            "route: payload type mismatch (expected demo.Pong)"
        );
        assert_eq!(
            RouteError::AmbiguousAddress.to_string(),
            "route: ambiguous Service(..) address not allowed here"
        );
        // DeadLetter 其余原因的可读性
        assert_eq!(
            DeadLetterReason::TargetNotAvailable(ServiceState::Error).to_string(),
            "target not available: error"
        );
        assert_eq!(
            DeadLetterReason::TypeMismatch.to_string(),
            "payload type mismatch"
        );
        assert_eq!(DeadLetterReason::NoSuchRoute.to_string(), "no such route");
        assert_eq!(DeadLetterReason::Other("boom").to_string(), "boom");
    }

    #[test]
    fn route_error_is_std_error() {
        fn assert_err<T: std::error::Error>() {}
        assert_err::<RouteError>();
    }
}
