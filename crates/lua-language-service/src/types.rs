//! 本 crate 的公开数据类型 (design D3: 后端中立 — 任何后端 crate 类型
//! 都不出现在这里, spec: Public API exposes backend-neutral types)。

use std::path::PathBuf;

/// 符号种类 (spec: Symbol search answers by name — kind 分类)。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    /// 函数 (`function` / 方法定义)。
    Function,
    /// 变量 (local / global 赋值)。
    Variable,
    /// table 字段 / 成员。
    Field,
    /// 模块 (文件即模块的模块级实体)。
    Module,
    /// 无法归类 (后端未给出可映射信息)。
    Unknown,
}

impl SymbolKind {
    /// 行文用短名 (view / MCP 序列化同源)。
    pub fn label(self) -> &'static str {
        match self {
            SymbolKind::Function => "fn",
            SymbolKind::Variable => "var",
            SymbolKind::Field => "field",
            SymbolKind::Module => "module",
            SymbolKind::Unknown => "?",
        }
    }
}

/// 一个符号命中 (spec: Symbol search answers by name)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SymbolHit {
    /// 符号名。
    pub name: String,
    /// 种类。
    pub kind: SymbolKind,
    /// 定义所在文件 (绝对路径)。
    pub file: PathBuf,
    /// 1-based 行号。
    pub line: u32,
    /// 1-based 列号。
    pub col: u32,
}

/// 诊断严重度 (spec: Workspace diagnostics — severity 由本 enum 承载)。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DiagnosticSeverity {
    /// 错误。
    Error,
    /// 警告。
    Warning,
    /// 提示 / 信息。
    Info,
    /// 无关紧要。
    Hint,
}

impl DiagnosticSeverity {
    /// 行文用短名。
    pub fn label(self) -> &'static str {
        match self {
            DiagnosticSeverity::Error => "error",
            DiagnosticSeverity::Warning => "warn",
            DiagnosticSeverity::Info => "info",
            DiagnosticSeverity::Hint => "hint",
        }
    }
}

/// 一条工作区诊断 (spec: Workspace diagnostics cover all indexed files)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LuaDiagnostic {
    /// 所在文件 (绝对路径)。
    pub file: PathBuf,
    /// 1-based 行号。
    pub line: u32,
    /// 1-based 列号。
    pub col: u32,
    /// 严重度。
    pub severity: DiagnosticSeverity,
    /// 诊断消息。
    pub message: String,
}

/// 索引阶段 (spec: Lua index section injects roots and shows index state)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexPhase {
    /// 从未注入 / 已清空。
    Empty,
    /// 索引进行中。
    InProgress,
    /// 索引完成, 可查询。
    Idle,
    /// 索引失败。
    Failed,
}

/// 索引状态快照 (view / 调试展示同源)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexStatus {
    /// 当前注入的 root 集 (全量替换后的集合)。
    pub roots: Vec<PathBuf>,
    /// 已索引文件数。
    pub file_count: usize,
    /// 最近一次索引耗时 (ms); 从未完成 = None。
    pub elapsed_ms: Option<u64>,
    /// 当前阶段。
    pub phase: IndexPhase,
    /// 失败时的行文 (phase != Failed 时为 None)。
    pub error: Option<String>,
}
