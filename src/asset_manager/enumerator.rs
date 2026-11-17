//! 资产枚举契约与标准实现 (design D2; spec: Asynchronous asset
//! enumeration / Module file enumeration with default exclusions)。
//!
//! - `AssetEnumeratable` → `AssetEnumerator`: 异步产出一棵资产子树;
//!   `BoxFuture` 承载对象安全的异步 (零新依赖, `Send` 边界保证可入后台)。
//! - Project 的枚举器 = 自身 → 异步枚举出所有 Module (其子节点)。
//! - Module 的枚举器 = 标准文件枚举器 (从 `module.root` 递归扫文件)。
//!
//! 扫描体 (排除表 / 符号链接不跟随 / 排序) 自旧 `filetree` 平移,
//! 语义逐字。

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use gpui_kit::BackgroundExecutor;

use crate::project_model::{Module, Project};

use super::dto::{AssetKind, AssetNode, project_id};

/// 异步枚举结果的盒装 future (BoxFuture 别名; 零新依赖)。
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// 可枚举资产的实体 (Project / Module)。
pub trait AssetEnumeratable {
    fn enumerator(&self) -> Box<dyn AssetEnumerator>;
}

/// 资产枚举器: 异步产出该实体的完整资产子树。
///
/// `bg` 供阻塞 fs 工作丢后台执行器; future 为 `'static`, 由 manager 在
/// 自己的 spawn 里驱动 (design D2: 一次 spawn 内组装, 原子 land)。
pub trait AssetEnumerator: Send {
    fn enumerate(
        self: Box<Self>,
        bg: BackgroundExecutor,
    ) -> BoxFuture<'static, Result<AssetNode, String>>;
}

/// spec 锚定的默认排除表 (asset-manager: Default exclusions)。
pub const EXCLUDES: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "build",
    "dist",
    ".DS_Store",
];

/// 标准文件资产枚举器 (单独实现): 从根目录全量递归枚举。
///
/// - 根不可读 → `Err` (manager 落降级态);
/// - 子目录不可读 → 跳过其内容 (best-effort, 目录仍显示);
/// - `file_type()` 不跟随符号链接: 链接按文件显示, 天然规避递归环;
/// - 排序: 目录在前、文件在后, 各自按标签 (大小写不敏感) 字典序。
pub struct FileEnumerator {
    pub root: PathBuf,
}

impl FileEnumerator {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

impl AssetEnumerator for FileEnumerator {
    fn enumerate(
        self: Box<Self>,
        bg: BackgroundExecutor,
    ) -> BoxFuture<'static, Result<AssetNode, String>> {
        let root = self.root;
        Box::pin(async move { bg.spawn(async move { scan_root(&root) }).await })
    }
}

/// 全量递归扫描 (后台执行; 模块根 → `Dir` 根节点)。
pub fn scan_root(root: &Path) -> Result<AssetNode, String> {
    if !root.is_dir() {
        return Err(format!("{} 不是可读目录", root.display()));
    }
    let children = scan_dir(root)?;
    Ok(AssetNode {
        id: root.to_string_lossy().to_string(),
        label: root
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.display().to_string()),
        kind: AssetKind::Dir,
        children,
    })
}

fn scan_dir(dir: &Path) -> Result<Vec<AssetNode>, String> {
    let entries =
        std::fs::read_dir(dir).map_err(|e| format!("read {}: {e}", dir.display()))?;
    let mut nodes = Vec::new();
    for entry in entries.flatten() {
        let label = entry.file_name().to_string_lossy().to_string();
        if EXCLUDES.contains(&label.as_str()) {
            continue;
        }
        let path = entry.path();
        // file_type = lstat: 符号链接不视为目录 (不递归, 防环)
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let id = path.to_string_lossy().to_string();
        if is_dir {
            let children = scan_dir(&path).unwrap_or_default(); // 子目录不可读 → 空目录仍显示
            nodes.push(AssetNode {
                id,
                label,
                kind: AssetKind::Dir,
                children,
            });
        } else {
            nodes.push(AssetNode {
                id,
                label,
                kind: AssetKind::File,
                children: Vec::new(),
            });
        }
    }
    nodes.sort_by(|a, b| {
        let rank = |n: &AssetNode| match n.kind {
            AssetKind::Dir | AssetKind::Module | AssetKind::Project => 0,
            AssetKind::File => 1,
        };
        rank(a)
            .cmp(&rank(b))
            .then_with(|| a.label.to_lowercase().cmp(&b.label.to_lowercase()))
            .then_with(|| a.label.cmp(&b.label))
    });
    Ok(nodes)
}

/// Module 枚举器: 标准文件枚举器从 `module.root` 扫描, 再以模块行
/// (label = 模块名, id = 模块根路径, kind = Module) 包一层 —
/// id 保持裸绝对路径, 目录行的菜单/复制语义直接成立 (design D3)。
struct ModuleEnumerator {
    module: Module,
}

/// Module 行组装 (纯函数): 扫描产物换壳为模块行。
/// id 保持裸绝对路径 (目录行菜单/复制语义), label = 模块名。
fn module_row(module: &Module, scanned: AssetNode) -> AssetNode {
    AssetNode {
        id: scanned.id,
        label: module.name.clone(),
        kind: AssetKind::Module,
        children: scanned.children,
    }
}

impl AssetEnumerator for ModuleEnumerator {
    fn enumerate(
        self: Box<Self>,
        bg: BackgroundExecutor,
    ) -> BoxFuture<'static, Result<AssetNode, String>> {
        let module = self.module;
        Box::pin(async move {
            let scanned = Box::new(FileEnumerator::new(module.root.clone()))
                .enumerate(bg)
                .await?;
            Ok(module_row(&module, scanned))
        })
    }
}

impl AssetEnumeratable for Module {
    fn enumerator(&self) -> Box<dyn AssetEnumerator> {
        Box::new(ModuleEnumerator {
            module: self.clone(),
        })
    }
}

/// Project 枚举器 = 自身: 异步枚举出所有 Module 作为子节点
/// (spec: Project enumerates its modules)。
///
/// v1 单 root module 顺序 await 即可; 子模块/子 Project 的嵌套与
/// "父排除子" 的重叠策略等有真实多模块 solution 时再接 (design Risks)。
struct ProjectEnumerator {
    project: Project,
}

/// Project 行组装 (纯函数): 子节点 = 各 Module 行。
fn project_row(project: &Project, module_nodes: Vec<AssetNode>) -> AssetNode {
    AssetNode {
        id: project_id(&project.root),
        label: project.name.clone(),
        kind: AssetKind::Project,
        children: module_nodes,
    }
}

impl AssetEnumerator for ProjectEnumerator {
    fn enumerate(
        self: Box<Self>,
        bg: BackgroundExecutor,
    ) -> BoxFuture<'static, Result<AssetNode, String>> {
        let project = self.project;
        Box::pin(async move {
            let mut modules = Vec::with_capacity(project.modules.len());
            for module in &project.modules {
                let node = module.enumerator().enumerate(bg.clone()).await?;
                modules.push(node);
            }
            Ok(project_row(&project, modules))
        })
    }
}

impl AssetEnumeratable for Project {
    fn enumerator(&self) -> Box<dyn AssetEnumerator> {
        Box::new(ProjectEnumerator {
            project: self.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每测试独立 temp 根 (进程 id + 名字, 平行安全)。
    fn temp_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "enumerator-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, "x").unwrap();
    }

    fn labels(nodes: &[AssetNode]) -> Vec<&str> {
        nodes.iter().map(|n| n.label.as_str()).collect()
    }

    fn find<'a>(nodes: &'a [AssetNode], label: &str) -> Option<&'a AssetNode> {
        nodes.iter().find(|n| n.label == label)
    }

    /// 平移自 filetree: 排除表 / dotfile / kind / 排序。
    #[test]
    fn file_enumerator_excludes_and_kinds() {
        let root = temp_root("scan");
        touch(&root.join("src/app.rs"));
        touch(&root.join("src/pane/mod.rs"));
        touch(&root.join("README.md"));
        touch(&root.join(".gitignore")); // dotfile 必须显示
        for excluded in EXCLUDES {
            touch(&root.join(format!("src/{excluded}/junk.js"))); // 任意深度排除
            touch(&root.join(excluded)); // 文件形态也排除 (.DS_Store 等)
        }
        touch(&root.join("src/deep/node_modules/dep/index.js"));

        let tree = scan_root(&root).expect("root 可读");
        assert_eq!(tree.kind, AssetKind::Dir);

        let top = labels(&tree.children);
        assert_eq!(
            top,
            vec!["src", ".gitignore", "README.md"],
            "顶层排序: 目录前, 大小写不敏感字典序; 排除项全部缺席"
        );

        fn assert_no_excluded(node: &AssetNode) {
            assert!(
                !EXCLUDES.contains(&node.label.as_str()),
                "排除项泄漏: {}",
                node.id
            );
            for child in &node.children {
                assert_no_excluded(child);
            }
        }
        assert_no_excluded(&tree);

        let src = find(&tree.children, "src").expect("src 在场");
        assert_eq!(src.kind, AssetKind::Dir);
        let app_rs = find(&src.children, "app.rs").expect("app.rs 在场");
        assert_eq!(app_rs.kind, AssetKind::File);
        let pane = find(&src.children, "pane").expect("pane 在场");
        assert_eq!(pane.kind, AssetKind::Dir);
        assert_eq!(labels(&pane.children), vec!["mod.rs"]);

        let _ = std::fs::remove_dir_all(&root);
    }

    /// 平移自 filetree: 根不可读 → Err (降级态输入)。
    #[test]
    fn file_enumerator_missing_root_errors() {
        let missing = std::env::temp_dir().join(format!(
            "enumerator-gone-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&missing);
        assert!(scan_root(&missing).is_err(), "根不存在 → Err");
        let file = temp_root("not-a-dir").join("plain.txt");
        touch(&file);
        assert!(scan_root(&file).is_err(), "非目录 → Err");
        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }

    /// 组装逻辑: 扫描产物换壳为 Module 行 (id=根裸路径, label=模块名)。
    #[test]
    fn module_row_wraps_scan_under_module_kind() {
        let root = temp_root("module-wrap");
        touch(&root.join("lib.rs"));
        let module = crate::project_model::Module::new("wrap", root.clone());

        let scanned = scan_root(&root).expect("可读");
        let node = module_row(&module, scanned);
        assert_eq!(node.id, root.to_string_lossy(), "id = 模块根裸路径");
        assert_eq!(node.label, "wrap");
        assert_eq!(node.kind, AssetKind::Module);
        assert!(find(&node.children, "lib.rs").is_some(), "子节点 = 扫描产物");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Project 行: id 命名空间前缀 (D3 补充), 子节点 = Module 行。
    /// (异步链的端到端由 manager 测试覆盖 — 2.2)
    #[test]
    fn project_row_id_is_namespaced_with_module_child() {
        let root = temp_root("project-row");
        touch(&root.join("main.rs"));
        let project = Project::from_root(root.clone());

        let scanned = scan_root(&root).expect("可读");
        let module_node = module_row(&project.modules[0], scanned);
        let node = project_row(&project, vec![module_node]);
        assert_eq!(node.id, project_id(&root));
        assert!(node.id.starts_with("project://"), "Project 行 id 带前缀");
        assert_ne!(node.id, node.children[0].id, "不得与 Module 行撞 id");
        assert_eq!(node.kind, AssetKind::Project);
        assert_eq!(node.label, project.name, "label = 项目名 (目录末段)");
        assert_eq!(node.children[0].kind, AssetKind::Module);
        let _ = std::fs::remove_dir_all(&root);
    }
}
