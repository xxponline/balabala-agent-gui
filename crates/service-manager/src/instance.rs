//! 实例/参与者表（design D7：状态挂每实例、未 load 不存在于实例表；
//! D3：路由表里没有特权参与者——宿主/GUI 同样注册为参与者）。
//!
//! 与 Catalog **结构分离**：`reload_catalog()` 只写 Catalog，本表不受影响（D8）。

use service_core::{ContractId, Envelope, InstanceId, ServiceName, ServiceState};
use std::collections::HashMap;
use tokio::sync::mpsc::Sender;

/// 路由表参与者条目：每参与者一个 per-outlet 有界通道（D6）。
pub struct Participant {
    pub id: InstanceId,
    /// `Some` = 服务实例（入实例表）；`None` = 宿主 actor / GUI（不入实例表）。
    pub service: Option<ServiceName>,
    /// 服务实例的契约身份（change `singleton-per-contract`, design D1）：
    /// 骨架单实例的类判据与拒载门索引键；宿主参与者恒 `None`。
    /// DLL 未解析过渡期可为 `None`（契约符号解析接入后恒 `Some`）。
    pub contract: Option<ContractId>,
    /// 服务实例状态；宿主参与者恒 `Available`（始终可投递）。
    pub state: ServiceState,
    /// per-outlet 有界通道发送端（Router `try_send`，满即刻失败，D6）。
    pub outlet: Sender<Envelope>,
}

impl Participant {
    /// 服务实例。
    pub fn service_instance(
        id: InstanceId,
        service: ServiceName,
        contract: Option<ContractId>,
        state: ServiceState,
        outlet: Sender<Envelope>,
    ) -> Self {
        Participant { id, service: Some(service), contract, state, outlet }
    }

    /// 宿主参与者（GUI/宿主 actor）：恒 available。
    pub fn host(id: InstanceId, outlet: Sender<Envelope>) -> Self {
        Participant { id, service: None, contract: None, state: ServiceState::Available, outlet }
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
    /// 契约 → 实例（拒载门的占用索引，design D4）：仅 `contract = Some`
    /// 的服务实例入索引；每契约至多一项（单实例的防御层）。
    by_contract: HashMap<ContractId, InstanceId>,
}

impl Instances {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, p: Participant) -> Option<Participant> {
        if let Some(c) = p.contract {
            match self.by_contract.get(&c) {
                Some(&other) if other != p.id => {
                    panic!("同契约仅允许一个服务实例（拒载门应在建实例前拦截, design D4）")
                }
                _ => {
                    self.by_contract.insert(c, p.id);
                }
            }
        }
        self.map.insert(p.id, p)
    }

    pub fn remove(&mut self, id: InstanceId) -> Option<Participant> {
        let p = self.map.remove(&id)?;
        if let Some(c) = p.contract
            && self.by_contract.get(&c) == Some(&id)
        {
            self.by_contract.remove(&c);
        }
        Some(p)
    }

    /// 契约占用查询（拒载门，design D4）：该契约已有实例则返回其 id。
    ///
    /// 功能：按契约身份查占位实例（任何状态都占位，含 `error`）。
    /// 输入：`contract` — 宿主铸造的契约身份。
    /// 输出：占用实例的 id；未占位返回 `None`。
    pub fn contract_instance(&self, contract: ContractId) -> Option<InstanceId> {
        self.by_contract.get(&contract).copied()
    }

    pub fn get(&self, id: InstanceId) -> Option<&Participant> {
        self.map.get(&id)
    }

    pub fn get_mut(&mut self, id: InstanceId) -> Option<&mut Participant> {
        self.map.get_mut(&id)
    }

    /// 服务名 → 唯一实例（change `singleton-per-contract` design D5：
    /// 单实例强制下 name 与实例 1:1，"挑选"词汇随之删除）。
    ///
    /// 功能：按服务名解析唯一实例 id（任意状态都解析，fail-fast 由路由层判）。
    /// 输入：`service` — 服务名。
    /// 输出：已装载实例 id；未装载返回 `None`。
    pub fn instance_of(&self, service: &ServiceName) -> Option<InstanceId> {
        self.map
            .values()
            .filter(|p| belongs(p, service))
            .map(|p| p.id)
            .min()
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
            None,
            ServiceState::Available,
            outlet(),
        ));
        t.insert(Participant::host(InstanceId(2), outlet()));

        assert_eq!(t.service_count(), 1, "宿主参与者不入实例表口径");
        assert_eq!(t.len(), 2, "但都在路由参与者表");
        assert_eq!(t.instance_of(&"demo".into()), Some(InstanceId(1)), "服务实例按名解析");
        assert_ne!(t.instance_of(&"demo".into()), Some(InstanceId(2)), "宿主参与者不占服务名");
    }

    /// 3.3: `instance_of` 按名解析——未装载/未注册名返回 `None`。
    ///
    /// 功能：断言按名解析的命中与未命中口径。
    /// 输入：无。
    /// 输出：断言通过。
    #[test]
    fn instance_of_returns_none_for_unknown_service() {
        let mut t = Instances::new();
        t.insert(Participant::service_instance(
            InstanceId(7),
            "demo".into(),
            None,
            ServiceState::Available,
            outlet(),
        ));
        assert_eq!(t.instance_of(&"other".into()), None, "未装载服务名无解析");
        assert_eq!(t.instance_of(&"demo".into()), Some(InstanceId(7)), "已装载名命中");
    }

    /// 测试 marker：契约 A。
    struct CA;
    /// 测试 marker：契约 B。
    struct CB;

    /// 1.3: 契约索引建立与卸载清除（拒载门的占用查询基础）。
    ///
    /// 功能：断言 insert 维护 `by_contract`、remove 同步清除、异契约互不占位。
    /// 输入：无。
    /// 输出：断言通过。
    #[test]
    fn contract_index_tracks_insert_and_remove() {
        let mut t = Instances::new();
        t.insert(Participant::service_instance(
            InstanceId(1),
            "demo".into(),
            Some(ContractId::of::<CA>()),
            ServiceState::Loading,
            outlet(),
        ));
        assert_eq!(t.contract_instance(ContractId::of::<CA>()), Some(InstanceId(1)));
        assert_eq!(t.contract_instance(ContractId::of::<CB>()), None, "异契约不占位");
        t.remove(InstanceId(1));
        assert_eq!(t.contract_instance(ContractId::of::<CA>()), None, "卸载清除索引");
    }

    /// 1.3: 同契约二次 insert 由防御断言拦截（装载门应在建实例前拒载）。
    ///
    /// 功能：断言同 `ContractId` 的第二个服务实例插入即 panic。
    /// 输入：无。
    /// 输出：`should_panic` 通过。
    #[test]
    #[should_panic(expected = "同契约仅允许一个服务实例")]
    fn second_insert_of_same_contract_panics() {
        let mut t = Instances::new();
        t.insert(Participant::service_instance(
            InstanceId(1),
            "demo".into(),
            Some(ContractId::of::<CA>()),
            ServiceState::Available,
            outlet(),
        ));
        t.insert(Participant::service_instance(
            InstanceId(2),
            "demo.other".into(),
            Some(ContractId::of::<CA>()),
            ServiceState::Available,
            outlet(),
        ));
    }
}
