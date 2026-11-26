//! 后端抽象 + emmylua 实现 (design D3: 私有 trait, 公开面不出后端类型)。
//!
//! [`Backend`] 是 crate 私有的换后端缝: 壳侧 (service.rs) 只认这四个操作
//! (建索引/喂文件/搜符号/取诊断), `emmylua_code_analysis` 的一切类型都
//! 关在本模块内 — 换后端 = 换本模块, 调用方零改动 (spec: Public API
//! exposes backend-neutral types)。

use std::path::{Path, PathBuf};

use emmy_lsp_types::DiagnosticSeverity as LspSeverity;
use emmylua_code_analysis::{
    EmmyLuaAnalysis, LuaDecl, WorkspaceFolder, collect_workspace_files, file_path_to_uri,
};
use emmylua_parser::LuaSyntaxKind;
use tokio_util::sync::CancellationToken;

use crate::types::{DiagnosticSeverity, LuaDiagnostic, SymbolHit, SymbolKind};

/// 待索引的单个文件 (壳中立: 路径 + 已读入的内容)。
pub(crate) struct SourceFile {
    pub path: PathBuf,
    pub content: String,
}

/// 换后端缝 (design D3): 壳侧只依赖这四个操作。
/// `Send + Sync`: 后端驻进程级 `RwLock` 状态 (D2), 需跨线程共享
/// (上游 `EmmyLuaAnalysis` 自带 unsafe Send+Sync 声明)。
pub(crate) trait Backend: Send + Sync {
    /// 为 roots 建全新后端 (std 库 + workspace roots 已注册), 并收集待索引文件。
    /// 校验失败 (root 不存在/非目录) 返回 `Err` → 壳侧置 Failed。
    fn build(roots: &[PathBuf]) -> Result<(Self, Vec<SourceFile>), String>
    where
        Self: Sized;

    /// 索引一批文件 (壳侧持写锁调用, 分批喂 → 读方可见部分状态)。
    fn index_chunk(&mut self, files: Vec<SourceFile>);

    /// 符号搜索 (spec: Symbol search answers by name — 精确/前缀/子串分级)。
    fn search(&self, query: &str) -> Vec<SymbolHit>;

    /// 已索引的 workspace 文件清单 (诊断扫描的目标; 已按 roots 过滤)。
    fn workspace_files(&self) -> Vec<PathBuf>;

    /// 单文件诊断 (spec: Workspace diagnostics — 壳侧逐文件持读锁调用,
    /// 文件间可被新 set_roots 打断 → 陈旧扫描可中止)。
    fn diagnose(&self, path: &Path) -> Vec<LuaDiagnostic>;
}

/// 建后端的唯一换后端接缝 (壳侧拿到即装箱)。
pub(crate) fn create_backend(
    roots: &[PathBuf],
) -> Result<(Box<dyn Backend>, Vec<SourceFile>), String> {
    EmmyluaBackend::build(roots).map(|(b, files)| (Box::new(b) as Box<dyn Backend>, files))
}

// ---------- emmylua 实现 ----------

struct EmmyluaBackend {
    analysis: EmmyLuaAnalysis,
    /// 注入的 roots — 搜索/诊断清单按此过滤 (std 库文件不在 roots 下, 天然排除)。
    roots: Vec<PathBuf>,
}

impl Backend for EmmyluaBackend {
    fn build(roots: &[PathBuf]) -> Result<(Self, Vec<SourceFile>), String> {
        for r in roots {
            if !r.is_dir() {
                return Err(format!("root is not a directory: {}", r.display()));
            }
        }
        let mut analysis = EmmyLuaAnalysis::new();
        // std 库 (资源已随 crate 内嵌, 无文件系统依赖) — 每次全量重建 (D2:
        // 全量替换语义下旧后端整体丢弃, std 重建是正确性前提)。
        analysis.init_std_lib(None);
        for r in roots {
            analysis.add_main_workspace(r.clone());
        }
        let emmyrc = analysis.get_emmyrc();
        let folders: Vec<WorkspaceFolder> = roots
            .iter()
            .map(|r| WorkspaceFolder::new(r.clone(), false))
            .collect();
        // collect 一次性读入全部内容 (fs 阶段不持锁, design D5: 重活在锁外)。
        let files = collect_workspace_files(&folders, &emmyrc, None, None)
            .into_iter()
            .map(|f| SourceFile {
                path: PathBuf::from(f.path),
                content: f.content,
            })
            .collect();
        Ok((
            Self {
                analysis,
                roots: roots.to_vec(),
            },
            files,
        ))
    }

    fn index_chunk(&mut self, files: Vec<SourceFile>) {
        let owned: Vec<(PathBuf, Option<String>)> = files
            .into_iter()
            .map(|f| (f.path, Some(f.content)))
            .collect();
        self.analysis.update_files_by_path(owned);
    }

    fn search(&self, query: &str) -> Vec<SymbolHit> {
        // 空 query = 列出全部 workspace 符号 (调试视图"注入即见";
        // rank_of 对空串走 prefix 分支, 按名排序)。
        let db = self.analysis.compilation.get_db();
        let vfs = db.get_vfs();
        let decl_index = db.get_decl_index();
        let module_index = db.get_module_index();
        let lower = query.to_lowercase();
        // (匹配级, hit) — 级别: 0 精确 / 1 前缀 / 2 子串 (大小写不敏感)。
        let mut ranked: Vec<(u8, SymbolHit)> = Vec::new();
        let rank_of = |name: &str| -> Option<u8> {
            let n = name.to_lowercase();
            if n == lower {
                Some(0)
            } else if n.starts_with(&lower) {
                Some(1)
            } else if n.contains(&lower) {
                Some(2)
            } else {
                None
            }
        };
        for fid in vfs.get_all_local_file_ids() {
            let Some(path) = vfs.get_file_path(&fid) else {
                continue;
            };
            if !self.roots.iter().any(|r| path.starts_with(r)) {
                continue; // 只出 workspace 符号 (std 库天然排除)
            }
            // 模块级命中 (文件即模块 — kind=Module, 位置=文件头)。
            if let Some(mi) = module_index.get_module(fid) {
                if let Some(rank) = rank_of(&mi.full_module_name).or_else(|| rank_of(&mi.name)) {
                    ranked.push((
                        rank,
                        SymbolHit {
                            name: mi.full_module_name.clone(),
                            kind: SymbolKind::Module,
                            file: path.clone(),
                            line: 1,
                            col: 1,
                        },
                    ));
                }
            }
            // 声明级命中 (每文件声明树线性扫 — design D4: 缓存后补, API 不变)。
            let Some(tree) = decl_index.get_decl_tree(&fid) else {
                continue;
            };
            let doc = vfs.get_document(&fid);
            for decl in tree.get_decls().values() {
                let name = decl.get_name();
                let Some(rank) = rank_of(name) else {
                    continue;
                };
                let (line, col) = doc
                    .as_ref()
                    .and_then(|d| d.get_line_col(decl.get_position()))
                    .map(|(l, c)| (l as u32 + 1, c as u32 + 1))
                    .unwrap_or((1, 1));
                ranked.push((
                    rank,
                    SymbolHit {
                        name: name.to_string(),
                        kind: classify(decl, db),
                        file: path.clone(),
                        line,
                        col,
                    },
                ));
            }
        }
        ranked.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| a.1.name.cmp(&b.1.name))
                .then_with(|| a.1.file.cmp(&b.1.file))
                .then_with(|| a.1.line.cmp(&b.1.line))
        });
        ranked.into_iter().map(|(_, h)| h).collect()
    }

    fn workspace_files(&self) -> Vec<PathBuf> {
        let db = self.analysis.compilation.get_db();
        let vfs = db.get_vfs();
        vfs.get_all_local_file_ids()
            .into_iter()
            .filter_map(|fid| vfs.get_file_path(&fid).cloned())
            .filter(|p| self.roots.iter().any(|r| p.starts_with(r)))
            .collect()
    }

    fn diagnose(&self, path: &Path) -> Vec<LuaDiagnostic> {
        let Some(uri) = file_path_to_uri(&path.to_path_buf()) else {
            return Vec::new();
        };
        let Some(fid) = self.analysis.get_file_id(&uri) else {
            return Vec::new();
        };
        // 壳侧的中止在文件粒度 (逐文件持读锁, 文件间查代际) — 这里的
        // token 只满足后端签名, 不承担跨文件取消。
        let Some(diags) = self.analysis.diagnose_file(fid, CancellationToken::new()) else {
            return Vec::new();
        };
        diags
            .into_iter()
            .map(|d| LuaDiagnostic {
                file: path.to_path_buf(),
                line: d.range.start.line + 1, // LSP 0-based → 1-based
                col: d.range.start.character + 1,
                severity: match d.severity {
                    Some(s) if s == LspSeverity::ERROR => DiagnosticSeverity::Error,
                    Some(s) if s == LspSeverity::WARNING => DiagnosticSeverity::Warning,
                    Some(s) if s == LspSeverity::INFORMATION => DiagnosticSeverity::Info,
                    Some(s) if s == LspSeverity::HINT => DiagnosticSeverity::Hint,
                    // 缺省 (LSP: 由客户端解释) 与未知新级别都归 Info
                    _ => DiagnosticSeverity::Info,
                },
                message: d.message,
            })
            .collect()
    }
}

/// 符号分类 (spec: kind — function/variable/module)。
///
/// function 判定两信号 (覆盖四种函数形态):
/// 1. 值是闭包 (`local f = function()` / `f = function()`);
/// 2. property 索引有 owner-map (`function f()` / `local function f()`,
///    二者均在建 decl 时登记 decl→签名映射)。
/// 已知边界: 带 doc 属性的非函数 decl 可能被误判为 function (v1 接受,
/// debug 视图场景影响可忽略)。
fn classify(decl: &LuaDecl, db: &emmylua_code_analysis::DbIndex) -> SymbolKind {
    if let Some(sid) = decl.get_value_syntax_id() {
        if sid.get_kind() == LuaSyntaxKind::ClosureExpr {
            return SymbolKind::Function;
        }
    }
    if db
        .get_property_index()
        .get_property(&emmylua_code_analysis::LuaSemanticDeclId::from(
            decl.get_id(),
        ))
        .is_some()
    {
        return SymbolKind::Function;
    }
    SymbolKind::Variable
}
