//! 实例/参与者表（design D7：状态挂每实例、未 load 不存在于实例表；
//! D3：路由表里没有特权参与者——宿主/GUI 同样注册为参与者）。
//!
//! 与 Catalog **结构分离**：`reload_catalog()` 只写 Catalog，本表不受影响（D8）。

use service_core::{Envelope, InstanceId, ServiceName, ServiceState};
use std::collections::HashMap;
use tokio::sync::mpsc::Sender;

/// 路由表参与者条目：每参与者一个 per-outlet 有界通道（D6）。
pub struct Participant {
    pub id: InstanceId,
    /// `Some` = 服务实例（入实例表）；`None` = 宿主 actor / GUI（不入实例表）。
    pub service: Option<ServiceName>,
    /// 服务实例状态；宿主参与者恒 `Available`（始终可投递）。
    pub state: ServiceState,
    /// per-outlet 有界通道发送端（Router `try_send`，满即刻失败，D6）。
    pub outlet: Sender<Envelope>,
}

impl Participant {
    /// 服务实例。
    pub fn service_instance(id: InstanceId, service: ServiceName, state: ServiceState, outlet: Sender<Envelope>) -> Self {
        Participant { id, service: Some(service), state, outlet }
    }

    /// 宿主参与者（GUI/宿主 actor）：恒 available。
    pub fn host(id: InstanceId, outlet: Sender<Envelope>) -> Self {
        Participant { id, service: None, state: ServiceState::Available, outlet }
    }

    /// 是否服务实例（即"存在于实例表"）。
    pub fn is_service_instance(&self) -> bool {
        self.service.is_some()
    }
}

/// 归属判断：参与者是该服务的服务实例。
fn belongs(p: &Participant, service: &ServiceName) -> bool {
    p.service.as_ref().is_some_and(|s| s.as_str() == service.as_str())
}

/// 实例表 + 参与者表（`service = Some` 即实例）。
#[derive(Default)]
pub struct Instances {
    map: HashMap<InstanceId, Participant>,
}

impl Instances {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, p: Participant) -> Option<Participant> {
        self.map.insert(p.id, p)
    }

    pub fn remove(&mut self, id: InstanceId) -> Option<Participant> {
        self.map.remove(&id)
    }

    pub fn get(&self, id: InstanceId) -> Option<&Participant> {
        self.map.get(&id)
    }

    pub fn get_mut(&mut self, id: InstanceId) -> Option<&mut Participant> {
        self.map.get_mut(&id)
    }

    /// 某服务的全部实例 id（仅服务实例）。
    pub fn instances_of(&self, service: &ServiceName) -> Vec<InstanceId> {
        let mut ids: Vec<_> = self
            .map
            .values()
            .filter(|p| belongs(p, service))
            .map(|p| p.id)
            .collect();
        ids.sort();
        ids
    }

    /// `Any` 策略：挑一个 `available` 实例（D4）。
    pub fn pick_available(&self, service: &ServiceName) -> Option<InstanceId> {
        let mut candidates: Vec<_> = self
            .map
            .values()
            .filter(|p| {
                belongs(p, service) && p.state == ServiceState::Available
            })
            .map(|p| p.id)
            .collect();
        candidates.sort();
        candidates.into_iter().next()
    }

    /// `Direct(id)` 断言：实例 id 必须属于该服务（D4 防误投）。
    pub fn is_instance_of(&self, id: InstanceId, service: &ServiceName) -> bool {
        self.map
            .get(&id)
            .is_some_and(|p| belongs(p, service))
    }

    /// 服务实例数（不含宿主参与者）。
    pub fn service_count(&self) -> usize {
        self.map.values().filter(|p| p.is_service_instance()).count()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn ids(&self) -> Vec<InstanceId> {
        let mut ids: Vec<_> = self.map.keys().copied().collect();
        ids.sort();
        ids
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    fn outlet() -> Sender<Envelope> {
        let (tx, _rx) = mpsc::channel(8);
        tx
    }

    #[test]
    fn service_instances_and_host_participants_coexist() {
        let mut t = Instances::new();
        t.insert(Participant::service_instance(
            InstanceId(1),
            "demo".into(),
            ServiceState::Available,
            outlet(),
        ));
        t.insert(Participant::host(InstanceId(2), outlet()));

        assert_eq!(t.service_count(), 1, "宿主参与者不入实例表口径");
        assert_eq!(t.len(), 2, "但都在路由参与者表");
        assert!(t.is_instance_of(InstanceId(1), &"demo".into()));
        assert!(!t.is_instance_of(InstanceId(2), &"demo".into()), "宿主不是服务实例");
    }

    #[test]
    fn pick_available_filters_by_state() {
        let mut t = Instances::new();
        t.insert(Participant::service_instance(
            InstanceId(1),
            "demo".into(),
            ServiceState::Loading,
            outlet(),
        ));
        assert_eq!(t.pick_available(&"demo".into()), None, "loading 不可选");

        t.insert(Participant::service_instance(
            InstanceId(2),
            "demo".into(),
            ServiceState::Available,
            outlet(),
        ));
        assert_eq!(t.pick_available(&"demo".into()), Some(InstanceId(2)));

        // 状态翻转后不可选
        t.get_mut(InstanceId(2)).unwrap().state = ServiceState::Error;
        assert_eq!(t.pick_available(&"demo".into()), None);

        assert!(t.instances_of(&"demo".into()).len() == 2, "instances_of 不看状态");
    }

    #[test]
    fn direct_policy_membership_check() {
        let mut t = Instances::new();
        t.insert(Participant::service_instance(
            InstanceId(7),
            "demo".into(),
            ServiceState::Available,
            outlet(),
        ));
        assert!(t.is_instance_of(InstanceId(7), &"demo".into()));
        assert!(!t.is_instance_of(InstanceId(7), &"other".into()), "服务名不符");
        assert!(!t.is_instance_of(InstanceId(99), &"demo".into()), "实例不存在");
    }
}
