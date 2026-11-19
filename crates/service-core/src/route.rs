//! 两级寻址（design D4）：`Route::Instance(..)` 与 `Route::Service(name, policy)`。

use crate::error::RouteError;
use std::fmt;

/// 实例 id：`load()` 分配，全框架单调递增，未 load 的服务不存在于实例表。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct InstanceId(pub u64);

/// ask 关联 id：一问一答凭它在 Router pending 表销账（**不按 from 寻址**，D5）。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct Corr(pub u64);

/// 服务名（Catalog 键）。
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ServiceName(pub std::sync::Arc<str>);

impl ServiceName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for ServiceName {
    fn from(s: &str) -> Self {
        ServiceName(std::sync::Arc::from(s))
    }
}

impl From<String> for ServiceName {
    fn from(s: String) -> Self {
        ServiceName(std::sync::Arc::from(s))
    }
}

impl fmt::Display for ServiceName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Debug for ServiceName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ServiceName({})", self.as_str())
    }
}

/// 服务名寻址策略（v1 只有 Direct 与 Any，round-robin 等真有负载需求再加）。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SelectPolicy {
    /// 携带实例 id 的精确指定 + "该实例必须属于该服务"的断言（防误投，D4）。
    Direct(InstanceId),
    /// 挑一个 `available` 实例——唯一"不知道目标是谁"的入口。
    Any,
}

/// 消息目标（两级寻址）。
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Route {
    /// 实例直呼：调用方拿着 `load()` 返回的实例 id 精确投递。
    Instance(InstanceId),
    /// 服务名寻址：Router 挑实例。
    Service(ServiceName, SelectPolicy),
}

/// `from` / 观测用的**受限地址**：只允许实例/actor 地址。
///
/// 类型层落实 design D5：`from` 不允许 `Service(..)` 歧义地址——回复与回报
/// 一律按 `corr` 在 Router pending 表销账，`from` 纯粹用于观测（"谁发的"）。
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Address(Route);

impl Address {
    /// 实例/actor 地址。
    pub fn instance(id: InstanceId) -> Self {
        Address(Route::Instance(id))
    }

    /// 底层路由（恒为 `Route::Instance(..)`）。
    pub fn route(&self) -> &Route {
        &self.0
    }

    /// 地址背后的实例 id。
    pub fn instance_id(&self) -> InstanceId {
        match &self.0 {
            Route::Instance(id) => *id,
            // 不变量: 构造只经 instance() / try_from(Instance(..))
            Route::Service(..) => unreachable!("Address invariant violated"),
        }
    }
}

impl TryFrom<Route> for Address {
    type Error = RouteError;

    fn try_from(route: Route) -> Result<Self, RouteError> {
        match route {
            Route::Instance(_) => Ok(Address(route)),
            Route::Service(..) => Err(RouteError::AmbiguousAddress),
        }
    }
}

impl From<Address> for Route {
    fn from(addr: Address) -> Route {
        addr.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_route_and_service_policy_construct() {
        let id = InstanceId(7);
        let direct = Route::Service("demo".into(), SelectPolicy::Direct(id));
        let any = Route::Service("demo".into(), SelectPolicy::Any);
        assert_eq!(
            direct,
            Route::Service("demo".into(), SelectPolicy::Direct(InstanceId(7)))
        );
        assert_ne!(direct, any);
        assert_eq!(Route::Instance(id), Route::Instance(InstanceId(7)));
    }

    #[test]
    fn address_accepts_instance_route() {
        let addr = Address::try_from(Route::Instance(InstanceId(3))).expect("实例地址合法");
        assert_eq!(addr.instance_id(), InstanceId(3));
        assert_eq!(addr.route(), &Route::Instance(InstanceId(3)));
        // 具名构造同路
        assert_eq!(Address::instance(InstanceId(3)), addr);
    }

    #[test]
    fn address_rejects_service_route() {
        // "from 拒绝 Service(..)" —— 语义断言 (task 2.1 验证点)
        let err = Address::try_from(Route::Service("demo".into(), SelectPolicy::Any))
            .expect_err("歧义地址不得作为 from");
        assert_eq!(err, RouteError::AmbiguousAddress);
        let err = Address::try_from(Route::Service(
            "demo".into(),
            SelectPolicy::Direct(InstanceId(1)),
        ))
        .expect_err("Direct 同样不得作为 from");
        assert_eq!(err, RouteError::AmbiguousAddress);
    }

    #[test]
    fn service_name_roundtrip() {
        let name = ServiceName::from("asset");
        assert_eq!(name.as_str(), "asset");
        assert_eq!(name.to_string(), "asset");
    }
}
