//! 每实例标准状态与状态翻转事件（design D7）。

use crate::route::{InstanceId, ServiceName};
use std::fmt;

/// 每实例状态（挂 supervisor，不是全局；未 load 的服务不存在于实例表）。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ServiceState {
    /// 装载/初始化中。
    Loading,
    /// 可服务——唯一接受消息的状态（fail-fast 语义，D6）。
    Available,
    /// 主动停用（可 enable 恢复）。
    Offline,
    /// 起不来 / 跑挂了（可 retry 或删）。
    Error,
}

impl fmt::Display for ServiceState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ServiceState::Loading => "loading",
            ServiceState::Available => "available",
            ServiceState::Offline => "offline",
            ServiceState::Error => "error",
        })
    }
}

/// 状态翻转事件：payload = 本类型，`kind` = [`crate::KIND_STATE_CHANGE`]。
///
/// 订阅塌缩成路由（D7/D9）：`subscribe_state(id)` = 往路由表加一条
/// "该实例的翻转事件投到我地址"，事件本体即此结构。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StateChange {
    pub instance: InstanceId,
    pub service: ServiceName,
    pub from: ServiceState,
    pub to: ServiceState,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_display_is_lowercase_vocabulary() {
        assert_eq!(ServiceState::Loading.to_string(), "loading");
        assert_eq!(ServiceState::Available.to_string(), "available");
        assert_eq!(ServiceState::Offline.to_string(), "offline");
        assert_eq!(ServiceState::Error.to_string(), "error");
    }

    #[test]
    fn state_change_carries_full_context() {
        let change = StateChange {
            instance: InstanceId(1),
            service: "demo".into(),
            from: ServiceState::Loading,
            to: ServiceState::Available,
        };
        assert_eq!(change.from, ServiceState::Loading);
        assert_eq!(change.to, ServiceState::Available);
        assert_eq!(change.service.as_str(), "demo");
    }
}
