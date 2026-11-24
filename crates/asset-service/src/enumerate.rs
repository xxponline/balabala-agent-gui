//! 枚举契约与枚举实现（自 `src/project_model/enumerate.rs`、
//! `src/assets/traits.rs` 迁入 —— change asset-service task 2.4）。
//!
//! ## 契约（`EnumeratorOwner` / `Enumerator`）
//!
//! 本文件前半是**枚举契约**：资产侧唯一的资产源视图（design D2）。
//! 链上不携带过滤输入 — 过滤规则由 Project 在枚举需要 Module 时构建并
//! 注入（design D2/D8），所有输入从 `root` 自身推导。
//!
//! ## design D4：签名不带执行器
//!
//! `Enumerator::enumerate` **不再收 `gpui_kit::BackgroundExecutor`**。
//! 单 worker 的 `service-layer` 运行时上，阻塞活必须由**调用方**决定
//! 丢到哪：
//!
//! - GUI 侧（`AssetManager::spawn_enumerate`）→ `cx.background_executor()`
//! - 服务侧（`AssetService`）→ `ServiceCtx::messaging::spawn_task`
//!
//! 契约只声明"产出子树"，executor 由调用方注入 — 服务因此不必认识
//! GPUI 执行器，GUI 也不必认识 tokio。

use std::path::Path;

use crate::contract::{
    AssetKind, AssetNode, ModuleRegistration, ProjectRegistration, project_id, view_row_actions,
};

/// 可枚举资产的实体 — 资产侧唯一的资产源视图（design D2）。
///
/// 链上不携带过滤输入 — 过滤规则由 Project 在枚举需要 Module 时
/// 构建并注入 (design D2/D8), 所有输入从 `root` 自身推导。
pub trait EnumeratorOwner {
    /// 身份根路径: 注册记账 / reveal 语义的依据。
    ///
    /// 输入: 无 (`&self`); 输出: 资产树根目录的绝对路径 (Project 根 /
    /// 模块根)。
    fn root(&self) -> &Path;

    /// 为该实体构造枚举执行体 (同步构造, 由调用方丢入自己的执行器)。
    ///
    /// 输入: 无 (`&self`); 输出: 异步产出该实体资产子树的枚举器。
    fn enumerator(&self) -> Box<dyn Enumerator>;
}

/// 资产枚举器: 产出该实体的完整资产子树。
///
/// **同步签名，不带执行器参数**（design D4 修订）——
/// `scan_root` 是同步 `std::fs::read_dir`，包一层 async 签名只是线程
/// 切换的载体、并不让 IO 变异步。改同步后 **offload 彻底归调用方**：
///
/// - GUI 侧 → `cx.background_executor().spawn(async move { en.enumerate() })`
/// - 服务侧 → `spawn_task(async { tokio::task::spawn_blocking(move || …) })`
///
/// 服务侧必须走 `spawn_blocking` —— 单 worker 上的协作式交错**扛不住
/// 同步 fs**（spec: Enumeration does not delay other services）。
pub trait Enumerator: Send {
    /// 执行枚举（消耗执行体；**同步阻塞**，调用方决定丢到哪个执行器）。
    ///
    /// 输入: 无; 输出: 资产子树 (`Err` 原因由调用方落降级态)。
    fn enumerate(self: Box<Self>) -> Result<AssetNode, String>;
}

// ================================================================================
// 枚举实现（自 `src/project_model/enumerate.rs` 迁入 — task 2.5）
// ================================================================================

use std::path::PathBuf;
use std::sync::Arc;

use crate::filter::{DotFilter, FileFilter, GitIgnoreFilter};

/// 标准文件资产枚举器 (单独实现): 保存注入的 filter 列表
/// (design D2), 从根目录递归枚举、逐条目过滤减支。
///
/// - 根不可读 → `Err` (manager 落降级态);
/// - 子目录不可读 → 跳过其内容 (best-effort, 目录仍显示);
/// - `file_type()` 不跟随符号链接: 链接按文件显示, 天然规避递归环;
/// - 过滤: `filters` 无序 OR, 任一命中即排除; 目录命中即不 `read_dir`;
/// - 排序: 目录在前、文件在后, 各自按标签 (大小写不敏感) 字典序。
pub struct FileEnumerator {
    pub root: PathBuf,
    filters: Vec<Arc<dyn FileFilter>>,
}

impl FileEnumerator {
    /// 注入建好的 filter 列表 (主路径: `module.filters` 克隆;
    /// 测试/自定义组合直接给列表 — design D2)。
    pub fn with_filters(root: impl Into<PathBuf>, filters: Vec<Arc<dyn FileFilter>>) -> Self {
        Self {
            root: root.into(),
            filters,
        }
    }
}

impl Enumerator for FileEnumerator {
    /// **同步**（design D4 修订）— offload 由调用方决定。
    fn enumerate(self: Box<Self>) -> Result<AssetNode, String> {
        scan_root(&self.root, &self.filters)
    }
}

/// 递归扫描 (后台执行; 模块根 → `Dir` 根节点), 按 `filters` 过滤减支。
pub fn scan_root(
    root: &Path,
    filters: &[Arc<dyn FileFilter>],
) -> Result<AssetNode, String> {
    if !root.is_dir() {
        return Err(format!("{} 不是可读目录", root.display()));
    }
    let children = scan_dir(root, "", filters)?;
    Ok(AssetNode {
        id: root.to_string_lossy().to_string(),
        label: root
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.display().to_string()),
        kind: AssetKind::Dir,
        children,
        actions: view_row_actions(),
    })
}

fn scan_dir(
    dir: &Path,
    rel_prefix: &str,
    filters: &[Arc<dyn FileFilter>],
) -> Result<Vec<AssetNode>, String> {
    let entries =
        std::fs::read_dir(dir).map_err(|e| format!("read {}: {e}", dir.display()))?;
    let mut nodes = Vec::new();
    for entry in entries.flatten() {
        let label = entry.file_name().to_string_lossy().to_string();
        // rel = 枚举根相对路径, '/' 连接 (design D7 逐层前缀)
        let rel = if rel_prefix.is_empty() {
            label.clone()
        } else {
            format!("{rel_prefix}/{label}")
        };
        // file_type = lstat: 符号链接不视为目录 (不递归, 防环)
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        // 无序 OR: 任一 filter 命中即排除 (默认 dot 排除是独立硬
        // filter — design D5; 目录命中即减支, 不 read_dir。
        if filters.iter().any(|f| f.is_ignored(&rel, is_dir)) {
            continue;
        }
        let path = entry.path();
        let id = path.to_string_lossy().to_string();
        if is_dir {
            let children = scan_dir(&path, &rel, filters).unwrap_or_default(); // 子目录不可读 → 空目录仍显示
            nodes.push(AssetNode {
                id,
                label,
                kind: AssetKind::Dir,
                children,
                actions: view_row_actions(),
            });
        } else {
            nodes.push(AssetNode {
                id,
                label,
                kind: AssetKind::File,
                children: Vec::new(),
                actions: view_row_actions(),
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
    module: ModuleRegistration,
    /// 由 [`ProjectEnumerator`] 在**本次枚举时**供给（design D3 修订：
    /// 过滤集不是快照，是每次枚举现场读 `.gitignore` 的结果）。
    filters: Vec<Arc<dyn FileFilter>>,
}

impl ModuleEnumerator {
    fn with_filters(module: ModuleRegistration, filters: Vec<Arc<dyn FileFilter>>) -> Self {
        Self { module, filters }
    }
}

/// Module 行组装 (纯函数): 扫描产物换壳为模块行。
/// id 保持裸绝对路径 (目录行菜单/复制语义), label = 模块名。
fn module_row(module: &ModuleRegistration, scanned: AssetNode) -> AssetNode {
    AssetNode {
        id: scanned.id,
        label: module.name.clone(),
        kind: AssetKind::Module,
        children: scanned.children,
        // Module 是可菜单行（旧 `is_menu_row` 语义）—— 与 Dir/File 同批声明
        actions: scanned.actions,
    }
}

impl Enumerator for ModuleEnumerator {
    /// **同步**（design D4 修订）— 子枚举直接 `.enumerate()?`。
    fn enumerate(self: Box<Self>) -> Result<AssetNode, String> {
        let scanned = Box::new(FileEnumerator::with_filters(
            self.module.root.clone(),
            self.filters,
        ))
        .enumerate()?;
        Ok(module_row(&self.module, scanned))
    }
}

/// Project 枚举器 = 自身: 异步枚举出所有 Module 作为子节点
/// (spec: Project enumerates its modules)。
///
/// v1 单 root module 顺序 await 即可; 子模块/子 Project 的嵌套与
/// "父排除子" 的重叠策略等有真实多模块 solution 时再接 (design Risks)。
struct ProjectEnumerator {
    project: ProjectRegistration,
}

/// Project 行组装 (纯函数): 子节点 = 各 Module 行。
fn project_row(project: &ProjectRegistration, module_nodes: Vec<AssetNode>) -> AssetNode {
    AssetNode {
        id: project_id(&project.root),
        label: project.name.clone(),
        kind: AssetKind::Project,
        children: module_nodes,
        // spec: … SHALL declare no actions on project rows ⇒ 右键无菜单
        actions: Arc::from([]),
    }
}

impl Enumerator for ProjectEnumerator {
    /// **同步**（design D4 修订）— 递归子枚举直接 `.enumerate()?`。
    fn enumerate(self: Box<Self>) -> Result<AssetNode, String> {
        let project = self.project;
        // ---- 过滤集在此**现场供给**（design D3 修订）----
        // 每次枚举重读 `root/.gitignore` ⇒ ⟳ 天然生效
        // (spec: Refresh re-reads filter rules)。git 判定用 DTO 带下的
        // `is_git`，**不探 `.git`** (spec: … instead of probing the filesystem)。
        let filters = project.provision_filters();
        let mut rows = Vec::with_capacity(project.modules.len());
        for module in &project.modules {
            let scanned = Box::new(ModuleEnumerator::with_filters(
                module.clone(),
                filters.clone(),
            ))
            .enumerate()?;
            rows.push(scanned);
        }
        Ok(project_row(&project, rows))
    }
}

impl ProjectRegistration {
    /// 按 `is_git` + `root` **现场**供给过滤集（design D3 修订；
    /// spec: Filter provisioning from the project root）。
    ///
    /// - 恒含默认 [`DotFilter`]（任意深度 `.` 前缀硬排除）
    /// - 仅当 `is_git`（root 是 git 目录 **且** solution 开 git 管理 ——
    ///   解析已在 solution 侧完成并随 DTO 带下）时读 `root/.gitignore`
    /// - **不探测 `.git`**；读失败 best-effort 跳过
    pub(crate) fn provision_filters(&self) -> Vec<Arc<dyn FileFilter>> {
        let mut filters: Vec<Arc<dyn FileFilter>> = vec![Arc::new(DotFilter)];
        if self.is_git
            && let Ok(text) = std::fs::read_to_string(self.root.join(".gitignore"))
        {
            filters.push(Arc::new(GitIgnoreFilter::from_text(&text)));
        }
        filters
    }
}

impl EnumeratorOwner for ProjectRegistration {
    fn root(&self) -> &Path {
        &self.root
    }

    fn enumerator(&self) -> Box<dyn Enumerator> {
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

    /// 默认 filter 列表 — 非 git 项目的供给形态 (design D8: 仅 Dot)。
    fn default_filters() -> Vec<Arc<dyn FileFilter>> {
        vec![Arc::new(DotFilter)]
    }

    /// design D2: FileEnumerator 保存注入的 filter 列表
    /// (主路径 `module.filters`; 组合由供给方决定)。
    #[test]
    fn file_enumerator_stores_injected_filters() {
        let e = FileEnumerator::with_filters(
            "/tmp",
            vec![
                Arc::new(DotFilter),
                Arc::new(GitIgnoreFilter::compile(&owned(&["*.log"]))),
            ],
        );
        assert_eq!(e.filters.len(), 2, "注入列表原样保存");
        assert!(
            e.filters.iter().any(|f| f.is_ignored(".env", false)),
            "dot filter 在列"
        );
        assert!(
            e.filters.iter().any(|f| f.is_ignored("a.log", false)),
            "规则 filter 在列"
        );
    }

    /// spec: Dot entries absent at any depth + Dependency directories
    /// present by default + kind/排序 (原 excludes 测试随六项撤销反转)。
    #[test]
    fn file_enumerator_hides_dot_entries_and_keeps_rest() {
        let root = temp_root("scan");
        touch(&root.join("src/app.rs"));
        touch(&root.join("src/pane/mod.rs"));
        touch(&root.join("README.md"));
        touch(&root.join(".gitignore")); // dot -> 缺席 (原 dotfile 显示已反转)
        touch(&root.join(".git/config")); // dot 目录整支缺席
        touch(&root.join("src/.DS_Store")); // 任意深度 dot 文件缺席
        touch(&root.join("src/.cache/x")); // 任意深度 dot 目录缺席
        touch(&root.join("node_modules/dep/index.js")); // 六项撤销 -> 在场
        touch(&root.join("target/debug/bin")); // 六项撤销 -> 在场

        let tree = scan_root(&root, &default_filters()).expect("root 可读");
        assert_eq!(tree.kind, AssetKind::Dir);

        let top = labels(&tree.children);
        assert_eq!(
            top,
            vec!["node_modules", "src", "target", "README.md"],
            "目录前大小写不敏感字典序; 依赖目录在场, dot 条目全部缺席"
        );

        fn assert_no_dot(node: &AssetNode) {
            assert!(!node.label.starts_with('.'), "dot 条目泄漏: {}", node.id);
            for child in &node.children {
                assert_no_dot(child);
            }
        }
        assert_no_dot(&tree);

        let src = find(&tree.children, "src").expect("src 在场");
        assert_eq!(src.kind, AssetKind::Dir);
        let app_rs = find(&src.children, "app.rs").expect("app.rs 在场");
        assert_eq!(app_rs.kind, AssetKind::File);
        let pane = find(&src.children, "pane").expect("pane 在场");
        assert_eq!(pane.kind, AssetKind::Dir);
        assert_eq!(labels(&pane.children), vec!["mod.rs"]);
        assert!(
            find(&src.children, ".DS_Store").is_none() && find(&src.children, ".cache").is_none(),
            "任意深度 dot 缺席"
        );

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
        assert!(scan_root(&missing, &default_filters()).is_err(), "根不存在 → Err");
        let file = temp_root("not-a-dir").join("plain.txt");
        touch(&file);
        assert!(scan_root(&file, &default_filters()).is_err(), "非目录 → Err");
        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }

    /// 测试夹具: DTO 构造（`Project`/`Module` 留在 `src/project_model`，
    /// 本 crate 只认 DTO — task 2.5）。root module 在首、name == project
    /// name（spec: Project and module structure shape）。
    fn module_reg(name: &str, root: PathBuf) -> ModuleRegistration {
        ModuleRegistration {
            name: name.to_string(),
            root,
            children: Vec::new(),
        }
    }

    fn project_reg(root: PathBuf) -> ProjectRegistration {
        let name = root
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "/".to_string());
        ProjectRegistration {
            name: name.clone(),
            root: root.clone(),
            is_git: false,
            modules: vec![module_reg(&name, root)],
        }
    }

    /// 组装逻辑: 扫描产物换壳为 Module 行 (id=根裸路径, label=模块名)。
    #[test]
    fn module_row_wraps_scan_under_module_kind() {
        let root = temp_root("module-wrap");
        touch(&root.join("lib.rs"));
        let module = module_reg("wrap", root.clone());

        let scanned = scan_root(&root, &default_filters()).expect("可读");
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
        let project = project_reg(root.clone());

        let scanned = scan_root(&root, &default_filters()).expect("可读");
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

    // ===== 减支 / 多 filter OR (spec: Enumeration prunes excluded directories) =====

    use crate::filter::{DotFilter, GitIgnoreFilter};
    use std::sync::{Arc, Mutex};

    fn owned(rules: &[&str]) -> Vec<String> {
        rules.iter().map(|s| s.to_string()).collect()
    }

    /// 默认 dot 排除 + 规则 (非 git 供给形态的手写等价, design D8)。
    fn ignore_filters(rules: &[&str]) -> Vec<Arc<dyn FileFilter>> {
        vec![
            Arc::new(DotFilter),
            Arc::new(GitIgnoreFilter::compile(&owned(rules))),
        ]
    }

    /// 减支证明的 spy: 记录每次求值, 并把 `build` 目录剪掉。
    struct SpyPruner {
        seen: Arc<Mutex<Vec<String>>>,
    }

    impl FileFilter for SpyPruner {
        fn is_ignored(&self, rel_path: &str, is_dir: bool) -> bool {
            self.seen.lock().unwrap().push(rel_path.to_string());
            rel_path == "build" && is_dir
        }
    }

    /// 递归断言: 树中无任何节点的 id/path 含 `needle`。
    fn assert_no_row(node: &AssetNode, needle: &str) {
        assert!(
            !node.id.contains(needle) && !node.label.contains(needle),
            "泄漏行: {}",
            node.id
        );
        for child in &node.children {
            assert_no_row(child, needle);
        }
    }

    /// spec: Excluded subtree is never enumerated — 被排除目录连同子树
    /// 无任何行 (含多个深度), 且 spy 对剪枝子树零求值 (证明不 read_dir)。
    #[test]
    fn pruning_skips_subtree_and_filter_never_asked_beneath() {
        let root = temp_root("prune-spy");
        touch(&root.join("build/top.js"));
        touch(&root.join("build/deep/inner.js"));
        touch(&root.join("build/deep/er/innermost.js"));
        touch(&root.join("keep/main.rs"));

        let seen = Arc::new(Mutex::new(Vec::new()));
        let filters: Vec<Arc<dyn FileFilter>> = vec![
            Arc::new(SpyPruner {
                seen: seen.clone(),
            }),
            Arc::new(DotFilter),
        ];
        let tree = scan_root(&root, &filters).expect("可读");

        let seen = seen.lock().unwrap();
        assert!(
            !seen.iter().any(|p| p.starts_with("build/")),
            "被剪枝子树零求值 (不 read_dir): {seen:?}"
        );
        assert!(seen.iter().any(|p| p == "build"), "目录自身被问到一次");
        assert!(
            seen.iter().any(|p| p.starts_with("keep")),
            "未剪枝分支正常求值"
        );
        drop(seen);
        assert_no_row(&tree, "build");
        assert!(find(&tree.children, "keep").is_some(), "未剪枝目录在场");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// OR 组合: 两个 filter 任一命中即排除, 与注入顺序无关。
    #[test]
    fn any_of_two_filters_excludes_entry_regardless_of_order() {
        let root = temp_root("or-two");
        touch(&root.join("a.txt"));
        touch(&root.join("b.txt"));
        touch(&root.join("c.txt"));

        for order in 0..2 {
            let f1 = GitIgnoreFilter::compile(&owned(&["a.txt"]));
            let f2 = GitIgnoreFilter::compile(&owned(&["b.txt"]));
            let mut filters: Vec<Arc<dyn FileFilter>> = if order == 0 {
                vec![Arc::new(f1), Arc::new(f2)]
            } else {
                vec![Arc::new(f2), Arc::new(f1)]
            };
            filters.push(Arc::new(DotFilter));
            let tree = scan_root(&root, &filters).expect("可读");
            assert_eq!(
                labels(&tree.children),
                vec!["c.txt"],
                "顺序 {order}: a/b 各自被不同的 filter 命中, 仅 c 在场"
            );
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// design D5: `.gitignore` 的 `!` 救不回默认 dot 排除 (OR 无跨
    /// filter 反悔) — spec: Dot ignore cannot be re-included。
    #[test]
    fn user_negation_cannot_rescue_dot_filter() {
        let root = temp_root("no-rescue");
        touch(&root.join(".env"));
        touch(&root.join("src/main.rs"));

        let filters = ignore_filters(&["!.env"]);
        let tree = scan_root(&root, &filters).expect("可读");
        assert!(
            find(&tree.children, ".env").is_none(),
            "DotFilter 的排除不可被另一 filter 的 `!` 反悔"
        );
        assert!(find(&tree.children, "src").is_some(), "其余照常");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// spec: Re-include beneath an excluded directory has no effect
    /// (spike 事实 3 — build 连同 keep.txt 整支缺席)。
    #[test]
    fn gotcha_reinclude_beneath_excluded_dir_absent() {
        let root = temp_root("gotcha");
        touch(&root.join("build/keep.txt"));
        touch(&root.join("build/other/x.js"));
        touch(&root.join("src/main.rs"));

        let filters = ignore_filters(&["build/", "!build/keep.txt"]);
        let tree = scan_root(&root, &filters).expect("可读");
        assert_no_row(&tree, "build");
        assert!(find(&tree.children, "src").is_some());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// spec: Re-including a directory re-includes its subtree。
    #[test]
    fn reinclude_directory_restores_subtree() {
        let root = temp_root("reinclude");
        touch(&root.join("src/a.rs"));
        touch(&root.join("src/deep/b.rs"));
        touch(&root.join("docs/d.md"));
        touch(&root.join("main.rs"));

        let filters = ignore_filters(&["*", "!src/", "!src/**"]);
        let tree = scan_root(&root, &filters).expect("可读");
        assert_eq!(labels(&tree.children), vec!["src"], "仅 src 生还");
        let src = find(&tree.children, "src").expect("src 在场");
        assert!(find(&src.children, "a.rs").is_some(), "子文件在场");
        let deep = find(&src.children, "deep").expect("孙目录在场");
        assert!(find(&deep.children, "b.rs").is_some(), "孙文件在场");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// spec: Unexcluded directory with all children filtered stays visible
    /// (design D6 空目录行保留)。
    #[test]
    fn unexcluded_dir_with_all_children_filtered_stays() {
        let root = temp_root("empty-dir");
        touch(&root.join("logs/a.log"));
        touch(&root.join("other/x.rs"));

        let filters = ignore_filters(&["*.log"]);
        let tree = scan_root(&root, &filters).expect("可读");
        let logs = find(&tree.children, "logs").expect("logs 目录行保留");
        assert!(logs.children.is_empty(), "子文件全被过滤 → 空行仍显示");
        assert!(find(&tree.children, "other").is_some());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// design D3: lstat 即 dtype — 符号链接按文件求值, `build/`
    /// (MUSTBEDIR) 不命中链接行。
    #[cfg(unix)]
    #[test]
    fn symlink_is_file_for_dir_only_rules() {
        use std::os::unix::fs::symlink;
        let root = temp_root("symlink");
        touch(&root.join("build/real.txt"));
        symlink(&root.join("build"), &root.join("link")).expect("symlink");

        let filters = ignore_filters(&["build/"]);
        let tree = scan_root(&root, &filters).expect("可读");
        assert!(find(&tree.children, "build").is_none(), "目录 build 剪枝");
        let link = find(&tree.children, "link").expect("链接行在场");
        assert_eq!(
            link.kind,
            AssetKind::File,
            "链接 = 文件 (lstat) → 目录专属规则不命中, 不递归"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// spec: Patterns are evaluated per project root — 同一批规则分别
    /// 对两个扫描根求值, 各自相对自己的根命中 (扫描期覆盖, 接替
    /// 已删除的后处理测试)。
    #[test]
    fn rules_evaluate_relative_to_each_scan_root() {
        for name in ["root-a", "root-b"] {
            let root = temp_root(name);
            touch(&root.join("src/x.rs"));
            touch(&root.join("docs/y.md"));

            let tree = scan_root(&root, &ignore_filters(&["src/**"])).expect("可读");
            let src = find(&tree.children, "src").expect("src 目录在 (src/** 不命中自身)");
            assert!(
                src.children.is_empty(),
                "{name}: src 子文件相对自身根命中 → 隐藏"
            );
            let docs = find(&tree.children, "docs").expect("docs 在");
            assert_eq!(docs.children.len(), 1, "{name}: 非 src 文件不受影响");
            let _ = std::fs::remove_dir_all(&root);
        }
    }

    /// spec: Deny-all with re-include — 仅根层 .rs 可见, 一切目录被
    /// 排除连同子树 (嵌套 .rs 不可达 = gotcha), 扫描期覆盖。
    #[test]
    fn deny_all_shows_only_root_level_rs() {
        let root = temp_root("deny-all");
        touch(&root.join("main.rs"));
        touch(&root.join("src/lib.rs"));
        touch(&root.join("docs/readme.md"));

        let tree = scan_root(&root, &ignore_filters(&["*", "!*.rs"])).expect("可读");
        assert_eq!(
            labels(&tree.children),
            vec!["main.rs"],
            "根层 .rs 生还; 目录全部剪枝, 嵌套 .rs 不可达"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    // ===== 过滤现场供给 (design D3 修订; spec: Filter provisioning) =====

    /// 每测试独立 temp 根 (进程 id + 名字, 平行安全)。
    fn prov_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("prov-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn prov_reg(root: PathBuf, is_git: bool) -> ProjectRegistration {
        ProjectRegistration {
            name: "p".into(),
            root,
            is_git,
            modules: Vec::new(),
        }
    }

    /// spec: Git-managed project applies its root gitignore。
    #[test]
    fn git_project_provisions_dot_and_gitignore() {
        let root = prov_root("git");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".gitignore"), "*.tmp\n").unwrap();
        let filters = prov_reg(root.clone(), true).provision_filters();
        assert!(filters.iter().any(|f| f.is_ignored(".env", false)), "默认 Dot 在列");
        assert!(
            filters.iter().any(|f| f.is_ignored("cache.tmp", false)),
            "根 .gitignore 生效 (spec 场景: cache.tmp 不出现)"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// spec: Non-git project does not read its gitignore。
    #[test]
    fn non_git_project_reads_nothing_but_dot() {
        let root = prov_root("non-git");
        std::fs::write(root.join(".gitignore"), "*.tmp\n").unwrap();
        let filters = prov_reg(root.clone(), false).provision_filters();
        assert!(filters.iter().any(|f| f.is_ignored(".env", false)), "默认 Dot 在列");
        assert!(
            !filters.iter().any(|f| f.is_ignored("cache.tmp", false)),
            "非 git 根的 .gitignore 不生效 (spec 场景: cache.tmp 仍出现)"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// git 项目但无 `.gitignore` → 只挂 Dot (读取 best-effort)。
    #[test]
    fn git_project_without_gitignore_only_dot() {
        let root = prov_root("git-empty");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        let filters = prov_reg(root.clone(), true).provision_filters();
        assert!(filters.iter().any(|f| f.is_ignored(".env", false)));
        assert!(
            !filters.iter().any(|f| f.is_ignored("notes.log", false)),
            "无 .gitignore → 无 gitignore 规则命中 (仅默认 Dot)"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// spec: Git directory with git management disabled does not read
    /// gitignore — `is_git` 由 DTO 带下, 关了就不读 (不探 `.git`)。
    #[test]
    fn git_dir_with_git_management_disabled_reads_no_gitignore() {
        let root = prov_root("git-disabled");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".gitignore"), "*.tmp\n").unwrap();
        let filters = prov_reg(root.clone(), false).provision_filters();
        assert!(filters.iter().any(|f| f.is_ignored(".env", false)), "默认 Dot 在列");
        assert!(
            !filters.iter().any(|f| f.is_ignored("cache.tmp", false)),
            "关 git 管理 ⇒ 有 .git/.gitignore 也不读 (不探文件系统)"
        );
        let _ = std::fs::remove_dir_all(&root);
    }


    /// task 6.1 (spec: File and directory rows declare view-executed
    /// actions): 可菜单行声明两条 **View 执行** 的动作; Project 行恒空。
    #[test]
    fn menu_rows_declare_two_view_executed_actions() {
        use contracts::shared::ExecutedBy;

        let root = prov_root("actions");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("a.rs"), "x").unwrap();

        // `project_reg` 带 root module —— `prov_reg` 是空模块集(仅供
        // provision_filters 单测), 这里要真的出树
        let reg = project_reg(root.clone());
        let tree = Box::new(ProjectEnumerator { project: reg }).enumerate().expect("枚举");

        // Project 行: 不声明任何动作 ⇒ 右键无菜单
        assert_eq!(tree.kind, AssetKind::Project, "根 = Project 行");
        assert!(
            tree.actions.is_empty(),
            "spec: SHALL declare no actions on project rows"
        );

        // Module 行 (可菜单行): 两条, 全 View 执行
        let module = &tree.children[0];
        assert_eq!(module.kind, AssetKind::Module);
        assert_eq!(module.actions.len(), 2, "module 行两条动作");
        for a in module.actions.iter() {
            assert_eq!(
                a.execution,
                ExecutedBy::View,
                "{} 必须标 View (design D5)",
                a.id
            );
        }
        let ids: Vec<&str> = module.actions.iter().map(|a| a.id.as_str()).collect();
        assert!(ids.contains(&crate::contract::ACTION_COPY_PATH), "含复制路径");
        assert!(ids.contains(&crate::contract::ACTION_REVEAL), "含 reveal");

        // 文件行: 同样声明
        let file = module
            .children
            .iter()
            .find(|n| n.label == "a.rs")
            .expect("文件行在树中");
        assert_eq!(file.actions.len(), 2, "文件行两条动作");
        assert!(
            file.actions.iter().all(|a| a.execution == ExecutedBy::View),
            "文件行也全 View 执行"
        );
    }

}
