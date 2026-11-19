//! 宿主侧构建指纹（task 3.2）：与插件侧 `service-plugin::build` 用**同一份
//! 算法**（service-core::fingerprint）计算，编译期固化为
//! `SERVICE_HOST_FINGERPRINT` 供 Loader 装载前比对（D8：不匹配显式拒载）。

fn main() {
    let manifest = std::path::PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"),
    );
    let contract_src = service_core::fingerprint::contract_src_from(&manifest);
    if let Some(src) = &contract_src {
        println!("cargo:rerun-if-changed={}", src.display());
    }
    let version = service_core::fingerprint::rustc_version().expect("rustc --version");
    let fp = service_core::fingerprint::compute(&version, contract_src.as_deref())
        .expect("compute host fingerprint");
    println!("cargo:rustc-env=SERVICE_HOST_FINGERPRINT={fp}");
    println!("cargo:rerun-if-changed=build.rs");
}
