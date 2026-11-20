//! Supervisor：每实例状态机 + 观测（design D7）。
//!
//! - **恢复策略单一权威**：本状态机是唯一恢复权威，kameo 自动 restart 禁用
//!   ——`apply` 是纯转移，不 spawn 任何东西；crash 只上报落 `error`，
//!   重建只能经显式 `Retry`（或 `Enable`）。
//! - 状态挂每实例（[`Instances`] 表），不是全局；未 load 的服务不存在于
//!   实例表（[`crate::Participant`] 都需先注册）。
//! - 每次翻转产出 [`StateChange`] 事件（`KIND_STATE_CHANGE`），经事件通道
//!   推送——订阅塌缩成路由的上游（D7/D9）。

use crate::instance::Instances;
use service_core::{InstanceId, ServiceState, StateChange};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

/// 状态动作（对应 D7 状态机的边）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    /// `loading` → `available`：init 成功。
    InitOk,
    /// `loading` → `error`：init 失败。
    InitErr,
    /// `available` → `offline`：主动停用。
    Disable,
    /// `offline` → `loading`：主动启用。
    Enable,
    /// `available` → `error`：actor crash（**只上报，不自动拉起**）。
    Crash,
    /// `error` → `loading`：显式重试。
    Retry,
}

impl std::fmt::Display for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Action::InitOk => "init ok",
            Action::InitErr => "init failed",
            Action::Disable => "disable",
            Action::Enable => "enable",
            Action::Crash => "crash",
            Action::Retry => "retry",
        })
    }
}

/// 转移被拒的原因。
#[derive(Debug, PartialEq, Eq)]
pub enum TransitionError {
    /// 参与者不存在，或不是服务实例（宿主参与者无状态机）。
    NoSuchInstance(InstanceId),
    /// 非法转移（不改状态、不发事件）。
    Illegal { from: ServiceState, action: Action },
    /// 删实例仅限 `error`（D7）。
    DeleteNotAllowed { from: ServiceState },
}

impl std::fmt::Display for TransitionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransitionError::NoSuchInstance(id) => write!(f, "no such service instance: {:?}", id),
            TransitionError::Illegal { from, action } => {
                write!(f, "illegal transition: {action} from {from}")
            }
            TransitionError::DeleteNotAllowed { from } => {
                write!(f, "delete only allowed from error, got {from}")
            }
        }
    }
}

impl std::error::Error for TransitionError {}

/// 每实例状态机：合法转移表见 D7 图；非法转移拒绝。
pub struct Supervisor {
    instances: Arc<Mutex<Instances>>,
    events: UnboundedSender<StateChange>,
}

impl Supervisor {
    /// 创建 supervisor 与事件流接收端（状态翻转的观测面）。
    pub fn new(instances: Arc<Mutex<Instances>>) -> (Self, UnboundedReceiver<StateChange>) {
        let (tx, rx) = unbounded_channel();
        (
            Supervisor {
                instances,
                events: tx,
            },
            rx,
        )
    }

    /// 执行一次状态翻转；成功产出 [`StateChange`] 事件（含服务名上下文）。
    ///
    /// 合法转移（D7）：
    /// - `loading` + InitOk → `available`；`loading` + InitErr → `error`
    /// - `available` + Disable → `offline`；`available` + Crash → `error`
    /// - `offline` + Enable → `loading`；`error` + Retry → `loading`
    pub fn apply(&self, id: InstanceId, action: Action) -> Result<StateChange, TransitionError> {
        let mut table = self.instances.lock().expect("instances poisoned");
        let (from, service) = {
            let p = table
                .get(id)
                .filter(|p| p.is_service_instance())
                .ok_or(TransitionError::NoSuchInstance(id))?;
            (p.state, p.service.clone().expect("service instance has name"))
        };
        let to = match (from, action) {
            (ServiceState::Loading, Action::InitOk) => ServiceState::Available,
            (ServiceState::Loading, Action::InitErr) => ServiceState::Error,
            (ServiceState::Available, Action::Disable) => ServiceState::Offline,
            (ServiceState::Available, Action::Crash) => ServiceState::Error,
            (ServiceState::Offline, Action::Enable) => ServiceState::Loading,
            (ServiceState::Error, Action::Retry) => ServiceState::Loading,
            (from, action) => {
                return Err(TransitionError::Illegal { from, action });
            }
        };
        table.get_mut(id).expect("checked above").state = to;
        let change = StateChange {
            instance: id,
            service,
            from,
            to,
        };
        // 观测面推送（订阅塌缩成路由的上游）; 接收端存活与否不影响状态机
        let _ = self.events.send(change.clone());
        Ok(change)
    }

    /// 删实例：**仅 `error` 可删**，从实例表移除（无状态翻转事件——"不存在"
    /// 不是状态，D7/D8 口径）。
    pub fn delete(&self, id: InstanceId) -> Result<(), TransitionError> {
        let mut table = self.instances.lock().expect("instances poisoned");
        let from = table
            .get(id)
            .filter(|p| p.is_service_instance())
            .ok_or(TransitionError::NoSuchInstance(id))?
            .state;
        if from != ServiceState::Error {
            return Err(TransitionError::DeleteNotAllowed { from });
        }
        table.remove(id);
        Ok(())
    }

    /// 当前状态（测试/观测）。
    pub fn state_of(&self, id: InstanceId) -> Option<ServiceState> {
        self.instances
            .lock()
            .expect("instances poisoned")
            .get(id)
            .filter(|p| p.is_service_instance())
            .map(|p| p.state)
    }

    /// 消息闸门前置（fail-fast 的状态判据，D6）：非 `available` 即拒。
    pub fn allows_delivery(&self, id: InstanceId) -> bool {
        self.state_of(id) == Some(ServiceState::Available)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_core::ServiceState;
    use tokio::sync::mpsc;

    fn setup() -> (Arc<Mutex<Instances>>, Supervisor, UnboundedReceiver<StateChange>) {
        let instances = Arc::new(Mutex::new(Instances::new()));
        let (sup, rx) = Supervisor::new(Arc::clone(&instances));
        (instances, sup, rx)
    }

    fn add_instance(table: &Mutex<Instances>, id: InstanceId, state: ServiceState) {
        let (tx, _rx) = mpsc::channel(8);
        table.lock().unwrap().insert(crate::instance::Participant::service_instance(
            id,
            "demo".into(),
            None,
            state,
            tx,
        ));
    }

    #[test]
    fn fresh_load_enters_loading() {
        let (table, _sup, _rx) = setup();
        add_instance(&table, InstanceId(1), ServiceState::Loading);
        // "load() → loading": 创建即 loading, 未 load 不存在于实例表
        let t = table.lock().unwrap();
        assert_eq!(t.get(InstanceId(1)).unwrap().state, ServiceState::Loading);
        assert!(t.get(InstanceId(2)).is_none(), "未 load 的服务不存在于实例表");
    }

    #[test]
    fn init_ok_reaches_available() {
        let (table, sup, mut rx) = setup();
        add_instance(&table, InstanceId(1), ServiceState::Loading);
        let change = sup.apply(InstanceId(1), Action::InitOk).unwrap();
        assert_eq!(change.from, ServiceState::Loading);
        assert_eq!(change.to, ServiceState::Available);
        assert_eq!(sup.state_of(InstanceId(1)), Some(ServiceState::Available));
        let evt = rx.try_recv().expect("应有事件");
        assert_eq!(evt, change, "事件与返回值一致");
    }

    #[test]
    fn init_failure_lands_in_error() {
        let (table, sup, _rx) = setup();
        add_instance(&table, InstanceId(1), ServiceState::Loading);
        sup.apply(InstanceId(1), Action::InitErr).unwrap();
        assert_eq!(sup.state_of(InstanceId(1)), Some(ServiceState::Error));
    }

    #[test]
    fn disable_then_enable_round_trip() {
        let (table, sup, mut rx) = setup();
        add_instance(&table, InstanceId(1), ServiceState::Available);
        sup.apply(InstanceId(1), Action::Disable).unwrap();
        assert_eq!(sup.state_of(InstanceId(1)), Some(ServiceState::Offline));
        sup.apply(InstanceId(1), Action::Enable).unwrap();
        assert_eq!(sup.state_of(InstanceId(1)), Some(ServiceState::Loading));
        // 事件按序到达
        let a = rx.try_recv().unwrap();
        let b = rx.try_recv().unwrap();
        assert_eq!((a.from, a.to), (ServiceState::Available, ServiceState::Offline));
        assert_eq!((b.from, b.to), (ServiceState::Offline, ServiceState::Loading));
    }

    #[test]
    fn crash_lands_in_error_without_auto_restart() {
        let (table, sup, _rx) = setup();
        add_instance(&table, InstanceId(1), ServiceState::Available);
        sup.apply(InstanceId(1), Action::Crash).unwrap();

        // 唯一权威 = 本状态机: crash 只落 error, 不 spawn 任何东西
        assert_eq!(sup.state_of(InstanceId(1)), Some(ServiceState::Error));
        {
            let t = table.lock().unwrap();
            assert_eq!(t.service_count(), 1, "没有新实例被自动创建 (不自动拉起)");
            assert_eq!(t.get(InstanceId(1)).unwrap().state, ServiceState::Error);
        }
        // fail-fast 前置: 非 available 一律拒 (路由层 4.2 据此拒绝)
        assert!(!sup.allows_delivery(InstanceId(1)), "crash 后消息 fail-fast");

        // 重建只能经显式 retry
        sup.apply(InstanceId(1), Action::Retry).unwrap();
        assert_eq!(sup.state_of(InstanceId(1)), Some(ServiceState::Loading));
        assert!(!sup.allows_delivery(InstanceId(1)), "retry 期间 (loading) 仍 fail-fast");
    }

    #[test]
    fn retry_reaches_available_via_init_ok() {
        let (table, sup, _rx) = setup();
        add_instance(&table, InstanceId(1), ServiceState::Error);
        sup.apply(InstanceId(1), Action::Retry).unwrap();
        sup.apply(InstanceId(1), Action::InitOk).unwrap();
        assert_eq!(sup.state_of(InstanceId(1)), Some(ServiceState::Available));
        assert!(sup.allows_delivery(InstanceId(1)));
    }

    #[test]
    fn illegal_transitions_are_rejected_without_side_effects() {
        let (table, sup, mut rx) = setup();
        add_instance(&table, InstanceId(1), ServiceState::Available);
        for action in [Action::InitOk, Action::InitErr, Action::Retry, Action::Enable] {
            let err = sup.apply(InstanceId(1), action).expect_err("available 下非法");
            assert_eq!(
                err,
                TransitionError::Illegal { from: ServiceState::Available, action }
            );
        }
        assert_eq!(sup.state_of(InstanceId(1)), Some(ServiceState::Available), "状态未变");
        assert!(rx.try_recv().is_err(), "无事件");

        // offline 不许 crash (已停用, 没有可崩的驱动)
        add_instance(&table, InstanceId(2), ServiceState::Offline);
        assert_eq!(
            sup.apply(InstanceId(2), Action::Crash).unwrap_err(),
            TransitionError::Illegal { from: ServiceState::Offline, action: Action::Crash }
        );

        // 非服务实例 (宿主参与者) 无状态机
        let (tx, _rx2) = mpsc::channel(8);
        table.lock().unwrap().insert(crate::instance::Participant::host(InstanceId(3), tx));
        assert_eq!(
            sup.apply(InstanceId(3), Action::InitOk).unwrap_err(),
            TransitionError::NoSuchInstance(InstanceId(3))
        );
        assert!(sup.apply(InstanceId(99), Action::InitOk).is_err());
    }

    #[test]
    fn delete_only_from_error() {
        let (table, sup, _rx) = setup();
        add_instance(&table, InstanceId(1), ServiceState::Available);
        assert_eq!(
            sup.delete(InstanceId(1)).unwrap_err(),
            TransitionError::DeleteNotAllowed { from: ServiceState::Available }
        );
        add_instance(&table, InstanceId(2), ServiceState::Error);
        sup.delete(InstanceId(2)).unwrap();
        assert!(sup.state_of(InstanceId(2)).is_none(), "已删实例不存在于表");
        assert_eq!(sup.delete(InstanceId(2)).unwrap_err(), TransitionError::NoSuchInstance(InstanceId(2)));
    }

    #[test]
    fn events_carry_service_context_in_order() {
        let (table, sup, mut rx) = setup();
        add_instance(&table, InstanceId(5), ServiceState::Loading);
        let c1 = sup.apply(InstanceId(5), Action::InitOk).unwrap();
        let c2 = sup.apply(InstanceId(5), Action::Disable).unwrap();
        assert_eq!(rx.try_recv().unwrap(), c1);
        assert_eq!(rx.try_recv().unwrap(), c2);
        assert_eq!(c1.service.as_str(), "demo");
        assert_eq!(c1.instance, InstanceId(5));
    }
}
