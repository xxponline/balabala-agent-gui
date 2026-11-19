//! 示例插件 DLL 端到端（tasks 5.1 / 5.2 / 3.3 的 DLL 断言；spec
//! `service-lifecycle` L3/L4、`service-routing` R1 的跨 dylib 载荷）。
//!
//! - 5.1：示例插件 cdylib 可被**扫描**并**装载**；
//! - 5.2：**D12 钉死**——Typed payload 真 dlopen round-trip（Ping 进、Pong 出，
//!   两侧 downcast 成功）；指纹不匹配产物**显式拒载**（实例表零写入）；
//! - 3.3：同 DLL 多实例 = **1 次 dlopen + N 次创建**；unload 移出实例表但
//!   **库仍驻留**（不 dlclose）。
//!
//! 构建脚本语义（task 5.2 的"plugins/ 产物构建"）：首次运行确保
//! `cargo build -p example-plugin` 并把产物拷进工作区 `plugins/`（扫描目录
//! 落点，产物 gitignore）。

use service_manager::{LoadError, ServiceManager, ServiceState};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

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

fn lib_file_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "example_plugin.dll"
    } else if cfg!(target_os = "linux") {
        "libexample_plugin.so"
    } else {
        "libexample_plugin.dylib"
    }
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent() // crates
        .and_then(|p| p.parent()) // workspace 根
        .expect("workspace root")
        .to_path_buf()
}

/// 确保示例插件已构建（缺则 `cargo build`），返回产物路径；
/// 并拷贝到工作区 `plugins/`（= 扫描目录落点）。
fn artifact_path() -> &'static PathBuf {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    PATH.get_or_init(|| {
        let root = workspace_root();
        let candidates = [
            root.join("target/debug").join(lib_file_name()),
            root.join("target/release").join(lib_file_name()),
        ];
        // 总是先构建 (缓存命中极快): 确保产物与当前源码同步 —— 否则会
        // dlopen 到旧版本脚手架 (已踩过: 旧 ActorService 仍在用 tokio::spawn)
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let status = std::process::Command::new(cargo)
            .current_dir(&root)
            .args(["build", "-p", "example-plugin"])
            .status()
            .expect("invoke cargo build -p example-plugin");
        assert!(status.success(), "example-plugin build failed");
        let artifact = candidates
            .iter()
            .find(|p| p.is_file())
            .cloned()
            .expect("example-plugin artifact");

        // 产物落点: 拷入 plugins/ (design D11 扫描目录; gitignored)
        let plugins = root.join("plugins");
        let _ = std::fs::create_dir_all(&plugins);
        let _ = std::fs::copy(&artifact, plugins.join(lib_file_name()));
        // 测试统一从自己的临时扫描目录装载 (隔离); 名字保持 lib 前缀供 scan 剥离
        artifact
    })
}

/// 把产物拷进临时扫描目录；`tamper = true` 时**等长改写**内嵌指纹
/// （只翻 `contracts:` 段的 16 hex 末位，不动文件布局）。
fn scan_dir_with_artifact(tag: &str, tamper: bool) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "smf-plugin-{tag}-{}-{:04x}",
        std::process::id(),
        (Instant::now().elapsed().as_nanos() ^ 0x1234) as u16
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let dest = dir.join(lib_file_name());
    std::fs::copy(artifact_path(), &dest).unwrap();
    if tamper {
        let mut bytes = std::fs::read(&dest).unwrap();
        let host_fp = service_manager::HOST_FINGERPRINT.as_bytes();
        let mut patch_count = 0;
        for start in find_all(&bytes, host_fp) {
            let rel = find_all(&bytes[start..], b"contracts:").first().copied();
            if let Some(rel) = rel {
                let hex_start = start + rel + b"contracts:".len();
                let hex_end = hex_start + 16;
                if hex_end <= bytes.len() {
                    let last = &mut bytes[hex_end - 1];
                    *last = if *last == b'0' { b'1' } else { b'0' };
                    patch_count += 1;
                }
            }
        }
        assert!(patch_count > 0, "必须改写到内嵌指纹: {}", service_manager::HOST_FINGERPRINT);
        std::fs::write(&dest, bytes).unwrap();
        // macOS: 篡改字节破坏 ad-hoc 签名 → dyld 会 SIGKILL, 重新签名
        #[cfg(target_os = "macos")]
        {
            let status = std::process::Command::new("codesign")
                .args(["--force", "--sign", "-"])
                .arg(&dest)
                .status()
                .expect("invoke codesign");
            assert!(status.success(), "ad-hoc 重签失败");
        }
    }
    dir
}

fn find_all(haystack: &[u8], needle: &[u8]) -> Vec<usize> {
    let mut out = Vec::new();
    if needle.is_empty() {
        return out;
    }
    let mut i = 0;
    while i + needle.len() <= haystack.len() {
        if &haystack[i..i + needle.len()] == needle {
            out.push(i);
            i += needle.len();
        } else {
            i += 1;
        }
    }
    out
}

fn spawn_two(dir: &PathBuf) -> (ServiceManager, service_manager::InstanceId, service_manager::InstanceId) {
    let mgr = ServiceManager::new(dir).expect("scan ok");
    assert!(
        mgr.has_service(&"example_plugin".into()),
        "扫描到示例插件 (5.1): {:?}",
        mgr.catalog_names()
    );
    let a = mgr.load(&"example_plugin".into()).expect("load a");
    let b = mgr.load(&"example_plugin".into()).expect("load b");
    assert!(
        wait_until(
            || {
                mgr.supervisor().state_of(a) == Some(ServiceState::Available)
                    && mgr.supervisor().state_of(b) == Some(ServiceState::Available)
            },
            Duration::from_secs(5)
        ),
        "两实例均 init 成功 (指纹过、kameo actor spawn 成功)"
    );
    (mgr, a, b)
}

fn caller(mgr: &ServiceManager) -> service_manager::Address {
    let id = {
        // allocate_id 是公开的
        mgr.allocate_id()
    };
    service_manager::Address::instance(id)
}

// ---------------- 5.1: 扫描 + 装载 ----------------

#[test]
fn task_5_1_example_plugin_scans_and_loads() {
    let dir = scan_dir_with_artifact("scan", false);
    let (mgr, _a, _b) = spawn_two(&dir);
    assert_eq!(mgr.instance_count(), 2);
    drop(mgr);
    std::fs::remove_dir_all(&dir).ok();
}

// ---------------- 5.2: D12 钉死 — Typed payload 跨 cdylib round-trip ----------------

#[test]
fn task_5_2_typed_payload_round_trip_across_dylib() {
    let dir = scan_dir_with_artifact("roundtrip", false);
    let (mgr, id, _other) = spawn_two(&dir);

    // Ping (contracts 类型) 作为 Typed payload 进插件, 插件 downcast 成功;
    // Pong 出插件, 宿主 downcast 成功 —— 两侧 TypeId 相等 (D12 甲路线钉死)
    use contracts::demo::{Ping, Pong};
    let env = service_manager::Envelope::new(
        service_manager::Route::Instance(id),
        caller(&mgr),
        "demo.ping",
        service_manager::Payload::new(Ping(7)),
    );
    let reply = futures::executor::block_on(mgr.call(env)).expect("ask 成功");
    let pong: &Pong = reply.downcast().expect("Pong 跨 dylib downcast 成功 (TypeId 相等)");
    assert_eq!(*pong, Pong(7), "值也原样透传");

    // 一问一答留痕 (corr 同一)
    let trace = mgr.trace_entries();
    let out = trace
        .iter()
        .find(|t| t.kind == "demo.ping" && t.direction == service_manager::TraceDirection::Out);
    let rep = trace
        .iter()
        .find(|t| t.kind == "demo.pong" && t.direction == service_manager::TraceDirection::Reply);
    assert_eq!(out.expect("请求留痕").corr, rep.expect("回复留痕").corr);

    // R1 跨 dylib 拒收: 故意发错类型 → 插件 ingress downcast 失败 →
    // messaging.reject → 死信 TypeMismatch (spec R1 场景, 跨边界版)
    let env = service_manager::Envelope::new(
        service_manager::Route::Instance(id),
        caller(&mgr),
        "demo.ping",
        service_manager::Payload::new("not a ping"),
    );
    let _ = futures::executor::block_on(mgr.call_with_timeout(env, Duration::from_millis(300)));
    assert!(
        mgr.dead_letters()
            .iter()
            .any(|d| matches!(
                d.reason,
                service_manager::DeadLetterReason::TypeMismatch
            )),
        "错类型跨边界拒收落死信: {:?}",
        mgr.dead_letters()
    );

    drop(mgr);
    std::fs::remove_dir_all(&dir).ok();
}

// ---------------- 5.2: 指纹不匹配 → 显式拒载 (spec L3) ----------------

#[test]
fn task_5_2_fingerprint_mismatch_refused_explicitly() {
    let dir = scan_dir_with_artifact("tamper", true);
    let mgr = ServiceManager::new(&dir).expect("scan ok");
    assert!(mgr.has_service(&"example_plugin".into()), "篡改仍可扫描 (校验在装载)");

    let err = mgr.load(&"example_plugin".into()).expect_err("指纹不匹配必须拒载");
    match &err {
        LoadError::FingerprintMismatch(_) => {}
        other => panic!("应是 FingerprintMismatch, got {other:?}"),
    }
    let text = err.to_string();
    assert!(text.contains("mismatch"), "报错信息含 mismatch: {text}");

    // 实例表零写入 (spec L3: no instance is created)
    assert_eq!(mgr.instance_count(), 0);
    assert!(mgr.supervisor().state_of(service_manager::InstanceId(1)).is_none());

    drop(mgr);
    std::fs::remove_dir_all(&dir).ok();
}

// ---------------- 3.3: 1 dlopen + N create; unload 不 dlclose (spec L4) ----------------

#[test]
fn task_3_3_dll_multi_instance_single_dlopen_unload_keeps_library() {
    let dir = scan_dir_with_artifact("multi", false);
    let (mgr, a, b) = spawn_two(&dir);
    assert_ne!(a, b);
    assert_eq!(mgr.instance_count(), 2);

    // 1 次 dlopen + N 次创建
    let libs = mgr.loaded_library_paths();
    assert_eq!(libs.len(), 1, "同一 DLL 只 dlopen 一次: {libs:?}");
    assert!(libs[0].ends_with(lib_file_name()));

    // 同库下多实例互不影响: 逐个 ask 都通
    use contracts::demo::{Ping, Pong};
    for id in [a, b] {
        let env = service_manager::Envelope::new(
            service_manager::Route::Instance(id),
            caller(&mgr),
            "demo.ping",
            service_manager::Payload::new(Ping(id.0)),
        );
        let reply = futures::executor::block_on(mgr.call(env)).expect("ask ok");
        let pong: &Pong = reply.downcast().unwrap();
        assert_eq!(pong.0, id.0, "回显自己的值");
    }

    // unload: 停用并移出实例表; 库仍驻留 (ManuallyDrop, 不 dlclose)
    mgr.unload(a).unwrap();
    mgr.unload(b).unwrap();
    assert_eq!(mgr.instance_count(), 0, "实例表无残留 (spec L4)");
    assert!(
        mgr.supervisor().state_of(a).is_none() && mgr.supervisor().state_of(b).is_none(),
        "supervisor 视角同样不存在"
    );
    let libs_after = mgr.loaded_library_paths();
    assert_eq!(libs_after.len(), 1, "库仍驻留, 未 dlclose (spec L4)");

    // Catalog 不受 unload 影响 (unload 不是 reload)
    assert!(mgr.has_service(&"example_plugin".into()));

    drop(mgr);
    std::fs::remove_dir_all(&dir).ok();
}
