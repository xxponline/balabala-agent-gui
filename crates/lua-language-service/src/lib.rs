//! `lua-language-service` — Lua 索引薄壳 (change add-lua-language-service)。
//!
//! 形态对齐提案 (proposal.md) 与设计 (design.md):
//! - **公开面只出本 crate 类型** (design D3): `SymbolHit` / `LuaDiagnostic` /
//!   `IndexStatus`, 后端 (`emmylua_code_analysis`) 藏在 crate 私有
//!   `Backend` trait 之后 — 换后端不动调用方 (spec: Public API exposes
//!   backend-neutral types)。
//! - **状态归进程级单例** (design D2): `OnceLock<RwLock<ServiceState>>` 持
//!   roots + 后端 + 索引状态; `set_roots` 全量替换 (spec: Root injection
//!   replaces the workspace set)。DebugView 关签不停索引。
//! - **分析在后台单 worker 线程** (design D5): `set_roots` / 诊断走 channel
//!   排队, 读方拿读锁 — UI 线程永不阻塞 (spec: Analysis runs off the UI
//!   thread)。

mod backend; // design D3: 私有换后端缝, 公开面不出后端类型
pub mod service;
pub mod types;
/// debug section 视图 — 随 debug_assertions 门控: release 构建的库不含
/// 任何 debug section 代码 (spec: no debug section code retained);
/// 主 crate 的 register_builtin 同门控, 两侧一致。
#[cfg(debug_assertions)]
pub mod view;

pub use service::{diagnostics_all, index_status, search_symbols, set_roots};
pub use types::{DiagnosticSeverity, IndexPhase, IndexStatus, LuaDiagnostic, SymbolHit, SymbolKind};
#[cfg(debug_assertions)]
pub use view::LuaIndexView;
