//! 端到端流程测试（task 4.1–4.4 验证）：Router/ask/超时/fail-fast/mailbox。
//!
//! 全部在 crate 内（可用 `pub(crate)` 测试接线）；测试线程**非 tokio 上下文**
//! ——这正是 executor-agnostic 的验证前提（D9）。

use crate::instance::Participant;
use crate::manager::ServiceManager;
use crate::router::Router;
use service_core::{
    Address, Envelope, InstanceId, Payload, Route, RouteError, SelectPolicy, ServiceState,
    StateChange, KIND_ROUTE_ERROR, KIND_STATE_CHANGE,
};
use std::time::{Duration, Instant};

/// 轮询等待（后台转发循环在 layer runtime 上跑，测试线程拉式等待）。
fn wait_until(mut cond: impl FnMut() -> bool, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if cond() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn test_manager() -> ServiceManager {
    let dir = std::env::temp_dir().join(format!(
        "smf-flow-{}-{:04x}",
        std::process::id(),
        (Instant::now().elapsed().as_nanos() ^ 0xabcd) as u16
    ));
    std::fs::create_dir_all(&dir).unwrap();
    ServiceManager::new(&dir).expect("manager")
}

/// 只收件不回复的参与者：(id, rx)。
fn silent_instance(
    mgr: &ServiceManager,
    service: &str,
    state: ServiceState,
    bound: usize,
) -> (InstanceId, tokio::sync::mpsc::Receiver<Envelope>) {
    let id = mgr.allocate_id();
    let (tx, rx) = tokio::sync::mpsc::channel(bound);
    mgr.instances.lock().unwrap().insert(Participant::service_instance(
        id,
        service.into(),
        state,
        tx,
    ));
    (id, rx)
}

/// 自动应答者：消费 ask 并回 `pong`（payload = `echo:{kind}`）。
fn spawn_echo_responder(mgr: &ServiceManager, service: &str) -> InstanceId {
    let id = mgr.allocate_id();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Envelope>(16);
    mgr.instances.lock().unwrap().insert(Participant::service_instance(
        id,
        service.into(),
        ServiceState::Available,
        tx,
    ));
    let handle = mgr.handle_for(id);
    let _ = mgr.spawn_bg(async move {
        while let Some(env) = rx.recv().await {
            if env.corr.is_some() {
                let _ = handle.reply(&env, "pong", Payload::new(format!("echo:{}", env.kind)));
            }
        }
    });
    id
}

fn caller_addr(mgr: &ServiceManager) -> Address {
    Address::instance(mgr.allocate_id())
}

// ---------------- task 4.1: 自建运行时 + executor-agnostic call ----------------

#[test]
fn task_4_1_call_awaitable_on_non_tokio_executor() {
    let mgr = test_manager();
    let responder = spawn_echo_responder(&mgr, "echo");

    let env = Envelope::new(
        Route::Instance(responder),
        caller_addr(&mgr),
        "ping",
        Payload::new(7u32),
    );
    // 测试线程没有任何 tokio 上下文; futures::executor::block_on 直接 await
    let reply = futures::executor::block_on(mgr.call(env)).expect("ask 成功");
    let echoed: &String = reply.downcast().expect("类型化回复");
    assert_eq!(echoed, "echo:ping");
    assert_eq!(mgr.pending_count(), 0, "回复后 pending 销账");
}

// ---------------- task 4.2: Router 寻址与投递 ----------------

#[test]
fn task_4_2_instance_direct_delivery_reaches_listener() {
    let mgr = test_manager();
    let mut listener = mgr.new_mailbox(16);

    let env = Envelope::new(
        Route::Instance(listener.id()),
        caller_addr(&mgr),
        "hello",
        Payload::new(1u8),
    );
    mgr.router().send(env).expect("直投成功");

    assert!(listener.poll(), "poll 取到件");
    let items: Vec<_> = listener.drain().collect();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].kind, "hello");
    assert!(!listener.poll(), "队列空后 poll 归 false");
}

#[test]
fn task_4_2_direct_with_wrong_service_name_is_refused() {
    let mgr = test_manager();
    let (id, _rx) = silent_instance(&mgr, "demo", ServiceState::Available, 4);

    let env = Envelope::new(
        Route::Service("other".into(), SelectPolicy::Direct(id)),
        caller_addr(&mgr),
        "x",
        Payload::new(()),
    );
    let err = mgr.router().send(env).expect_err("服务名不符必须拒绝");
    assert_eq!(err, RouteError::NoSuchRoute, "Direct 断言失败 = 拒绝 (spec R2)");
    // 拒绝也留观测
    assert_eq!(mgr.dead_letters().len(), 1);
}

#[test]
fn task_4_2_any_picks_only_an_available_instance() {
    let mgr = test_manager();
    let (loading_id, mut loading_rx) = silent_instance(&mgr, "demo", ServiceState::Loading, 4);
    let (avail_id, mut avail_rx) = silent_instance(&mgr, "demo", ServiceState::Available, 4);

    let env = Envelope::new(
        Route::Service("demo".into(), SelectPolicy::Any),
        caller_addr(&mgr),
        "work",
        Payload::new(()),
    );
    mgr.router().send(env).expect("Any 有 available 即成功");

    assert!(avail_rx.try_recv().is_ok(), "投给 available 实例");
    assert!(loading_rx.try_recv().is_err(), "loading 实例不收件");
    let _ = (loading_id, avail_id);
}

#[test]
fn task_4_2_full_channel_fails_immediately_not_after_timeout() {
    let mgr = test_manager();
    let (id, _rx) = silent_instance(&mgr, "demo", ServiceState::Available, 1);

    // tell: 满即刻失败 + 死信
    let fill = Envelope::new(Route::Instance(id), caller_addr(&mgr), "t1", Payload::new(()));
    mgr.router().send(fill).expect("首件入队");
    let over = Envelope::new(Route::Instance(id), caller_addr(&mgr), "t2", Payload::new(()));
    let err = mgr.router().send(over).expect_err("队列满立即拒绝");
    assert_eq!(err, RouteError::DeadLetter(service_core::DeadLetterReason::ChannelFull));
    assert_eq!(mgr.dead_letters().len(), 1, "tell 满落死信");

    // ask: 满即刻失败 —— 不等 30s 超时
    let env = Envelope::new(Route::Instance(id), caller_addr(&mgr), "q", Payload::new(()));
    let started = Instant::now();
    let err = futures::executor::block_on(mgr.call_with_timeout(env, Duration::from_secs(30)))
        .expect_err("队列满 ask 必失败");
    assert_eq!(err, RouteError::DeadLetter(service_core::DeadLetterReason::ChannelFull));
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "调用方不干等超时, 实际耗时 {:?}",
        started.elapsed()
    );
    assert_eq!(mgr.pending_count(), 0, "失败即刻销账, 不挂 pending");
}

// ---------------- task 4.3: ask / 回复 / 超时 / fail-fast ----------------

#[test]
fn task_4_3_ask_reply_round_trip_leaves_trace() {
    let mgr = test_manager();
    let responder = spawn_echo_responder(&mgr, "echo");

    let env = Envelope::new(
        Route::Instance(responder),
        caller_addr(&mgr),
        "ping",
        Payload::new(()),
    );
    let reply = futures::executor::block_on(mgr.call(env)).expect("一问一答成功");
    assert!(reply.downcast::<String>().is_some());

    // 全程留痕: 请求 Out 与回复 Reply 同 corr 可查
    let trace = mgr.trace_entries();
    let out = trace.iter().find(|t| t.direction == crate::router::TraceDirection::Out && t.kind == "ping");
    let rep = trace.iter().find(|t| t.direction == crate::router::TraceDirection::Reply && t.kind == "pong");
    let out = out.expect("请求留痕");
    let rep = rep.expect("回复留痕");
    assert_eq!(out.corr, rep.corr, "一问一答同一 corr");
    assert!(out.corr.is_some());
    assert_eq!(mgr.dead_letters().len(), 0, "成功链路无死信");
}

#[test]
fn task_4_3_timeout_is_router_held_and_observable() {
    let mgr = test_manager();
    let (silent, _rx) = silent_instance(&mgr, "demo", ServiceState::Available, 4);

    let env = Envelope::new(
        Route::Instance(silent),
        caller_addr(&mgr),
        "no-reply",
        Payload::new(()),
    );
    let started = Instant::now();
    let err = futures::executor::block_on(mgr.call_with_timeout(env, Duration::from_millis(80)))
        .expect_err("无回复必超时");
    assert_eq!(err, RouteError::Timeout);
    assert!(started.elapsed() >= Duration::from_millis(70), "确实等了 deadline");

    // 超时可观测: 死信 + pending 销账
    let deads = mgr.dead_letters();
    assert_eq!(deads.len(), 1);
    assert_eq!(
        deads[0].reason,
        service_core::DeadLetterReason::Other("ask timeout")
    );
    assert!(deads[0].corr.is_some());
    assert_eq!(mgr.pending_count(), 0, "到期销账");

    // 迟到回复不会再有人收 (晚到 → 死信)
    let handle = mgr.handle_for(silent);
    let late = Envelope::new(Route::Instance(InstanceId(0)), handle.address().clone(), "late", Payload::new(()))
        .with_corr(deads[0].corr.unwrap());
    let err = mgr.router().send(late).expect_err("晚到回复无 pending");
    assert_eq!(err, RouteError::NoSuchRoute);
    assert_eq!(mgr.dead_letters().len(), 2, "晚到回复落死信");
}

#[test]
fn task_4_3_fail_fast_ask_and_tell() {
    let mgr = test_manager();
    let (loading, mut loading_rx) = silent_instance(&mgr, "demo", ServiceState::Loading, 4);
    let (errored, _rx) = silent_instance(&mgr, "demo", ServiceState::Error, 4);

    // ask 到 loading 目标 → 即刻 TargetNotAvailable (spec R4)
    let env = Envelope::new(Route::Instance(loading), caller_addr(&mgr), "q", Payload::new(()));
    let started = Instant::now();
    let err = futures::executor::block_on(mgr.call(env)).expect_err("loading 拒绝");
    assert_eq!(err, RouteError::TargetNotAvailable(ServiceState::Loading));
    assert!(started.elapsed() < Duration::from_secs(1), "立即拒绝, 不等超时");
    assert!(loading_rx.try_recv().is_err(), "零缓冲: loading 目标没收到任何件");

    // tell 到 error 目标 → 死信 (spec R4)
    let env = Envelope::new(Route::Instance(errored), caller_addr(&mgr), "t", Payload::new(()));
    let err = mgr.tell(env).expect_err("error 拒绝");
    assert_eq!(err, RouteError::TargetNotAvailable(ServiceState::Error));
    let deads = mgr.dead_letters();
    assert!(
        deads.iter().any(|d| matches!(d.reason, service_core::DeadLetterReason::TargetNotAvailable(ServiceState::Error))),
        "tell 失败落死信可观测"
    );
}

#[test]
fn task_4_3_submit_reply_lands_in_listener_mailbox() {
    let mgr = test_manager();
    let responder = spawn_echo_responder(&mgr, "echo");
    let mut listener = mgr.new_mailbox(16);

    // 正常回复落 mailbox (poll 门)
    let corr = listener
        .submit(Route::Instance(responder), "ping", Payload::new(()))
        .expect("submit 成功");
    assert!(wait_until(|| listener.poll(), Duration::from_secs(2)), "回复到达");
    let reply = listener.drain().find(|e| e.corr == Some(corr)).expect("按 corr 取回");
    assert_eq!(reply.kind, "pong");
    assert_eq!(
        reply.to,
        Route::Instance(listener.id()),
        "回复 to 重写为发起方 (匹配仍靠 corr)"
    );

    // 超时对 poll 调用方可见: route_error 信封 (spec: 宿主门错误可见性)
    let (silent, _rx) = silent_instance(&mgr, "demo", ServiceState::Available, 4);
    let corr = listener
        .submit_with_timeout(Route::Instance(silent), "q", Payload::new(()), Duration::from_millis(80))
        .expect("submit 成功");
    assert!(
        wait_until(|| listener.poll(), Duration::from_secs(2)),
        "超时信封到达"
    );
    let err_env = listener.drain().find(|e| e.corr == Some(corr)).expect("超时可见");
    assert_eq!(err_env.kind, KIND_ROUTE_ERROR);
    let err: &RouteError = err_env.downcast().expect("route_error payload");
    assert_eq!(*err, RouteError::Timeout);
}

// ---------------- task 4.4: Listener (poll 门) ----------------

#[derive(Debug, PartialEq, Eq)]
struct TestSnap(u64);

#[test]
fn task_4_4_snapshot_is_zero_roundtrip() {
    let mgr = test_manager();
    let mut listener = mgr.new_mailbox(16);

    // 变更事件自带快照 Arc —— 不需要任何回程 ask
    let env = Envelope::new(
        Route::Instance(listener.id()),
        caller_addr(&mgr),
        "asset.tree",
        Payload::new(TestSnap(42)),
    );
    mgr.router().send(env).unwrap();
    assert!(listener.poll());
    let snap = listener.snapshot::<TestSnap>().expect("snapshot 零回程");
    assert_eq!(snap.0, 42);

    // 同 kind 更新 → 最后一值生效
    let env = Envelope::new(
        Route::Instance(listener.id()),
        caller_addr(&mgr),
        "asset.tree",
        Payload::new(TestSnap(43)),
    );
    mgr.router().send(env).unwrap();
    assert!(listener.poll());
    assert_eq!(listener.snapshot::<TestSnap>().unwrap().0, 43);
    // 不同类型不串
    assert!(listener.snapshot::<String>().is_none());
}

#[test]
fn task_4_4_poll_coalesces_but_replies_stay_discrete() {
    let mgr = test_manager();
    let mut listener = mgr.new_mailbox(64);

    // 连续 3 次状态翻转事件 → poll 只亮一次 (合并)
    for i in 0..3u8 {
        let env = Envelope::new(
            Route::Instance(listener.id()),
            caller_addr(&mgr),
            KIND_STATE_CHANGE,
            Payload::new(i),
        );
        mgr.router().send(env).unwrap();
    }
    assert!(listener.poll(), "有新件 → true");
    assert!(!listener.poll(), "无新件 → false (N 次合并为一次 changed)");
    assert_eq!(listener.drain().count(), 3, "drain 逐条不丢");
    assert!(!listener.poll());

    // Reply 事件不合并: 两个 ask 的回复各自可配对
    let responder = spawn_echo_responder(&mgr, "echo");
    let c1 = listener
        .submit(Route::Instance(responder), "a", Payload::new(()))
        .unwrap();
    let c2 = listener
        .submit(Route::Instance(responder), "b", Payload::new(()))
        .unwrap();
    assert_ne!(c1, c2, "corr 各自独立");
    assert!(
        wait_until(|| listener.poll(), Duration::from_secs(2)),
        "两个回复都到"
    );
    let replies: Vec<_> = listener.drain().filter(|e| e.corr.is_some()).collect();
    assert_eq!(replies.len(), 2, "Reply 逐条, corr 语义不被合并破坏");
    let corrs: Vec<_> = replies.iter().map(|e| e.corr.unwrap()).collect();
    assert!(corrs.contains(&c1) && corrs.contains(&c2));
}

#[test]
fn task_4_4_subscription_rolls_into_routing() {
    let mgr = test_manager();
    let mut listener = mgr.new_mailbox(16);

    // 注册一个 loading 中的服务实例 (load 语义的表形态)
    let (id, _rx) = silent_instance(&mgr, "demo", ServiceState::Loading, 4);
    // 订阅塌缩成路由: = 往路由表加一条投递规则
    mgr.subscribe_state(id, &listener);

    mgr.supervisor().apply(id, crate::Action::InitOk).unwrap();

    assert!(
        wait_until(|| listener.poll(), Duration::from_secs(2)),
        "状态翻转事件送达订阅者 mailbox"
    );
    let evt = listener.drain().next().expect("有事件");
    assert_eq!(evt.kind, KIND_STATE_CHANGE);
    let change: &StateChange = evt.downcast().expect("StateChange payload");
    assert_eq!(change.instance, id);
    assert_eq!(change.from, ServiceState::Loading);
    assert_eq!(change.to, ServiceState::Available);
    assert_eq!(evt.to, Route::Instance(listener.id()));
}

// 依赖面守卫 (D11): manager 只经自身 re-export 暴露词汇, 无 contracts/GUI 依赖。
#[test]
fn router_and_manager_types_are_publicly_usable_without_tokio_names() {
    // 编译期断言: 宿主公共 API 不要求调用方命名任何 tokio 类型
    fn takes_public_types(
        _mgr: &ServiceManager,
        _listener: &crate::Listener,
        _router: &Router,
        _env: &Envelope,
        _payload: &Payload,
        _err: &RouteError,
    ) {
    }
    let _ = takes_public_types as fn(&ServiceManager, &crate::Listener, &Router, &Envelope, &Payload, &RouteError);
}
