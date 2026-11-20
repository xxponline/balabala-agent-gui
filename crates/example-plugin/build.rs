//! 示例插件构建脚本：嵌入契约构建指纹 + 契约名导出符号
//! （task 3.2 / design D8、D12 路线甲；change singleton-per-contract design D2）。
fn main() {
    service_plugin::build::emit_fingerprint();
    service_plugin::build::emit_contract_name("demo");
}
