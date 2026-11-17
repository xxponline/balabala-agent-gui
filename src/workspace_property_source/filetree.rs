//! 文件树数据源——`WorkspacePropertySource` 的第一个实现（插件家族首发）。
//!
//! part1 职责（design D6/D7）：
//! - 定位活动 workspace 根（`resolve_active_root`；通知路径的同根比对
//!   也收敛在此 — `refresh_if_needed`，D6）；
//! - 读活动工作区自己配置文件的 `file_tree` 规则区段
//!   （`.balabala.workspace.json`，单工作区不再按键控 — file-tree delta）；
//! - 全量递归扫描（默认排除表 + 后台线程），落权威树；
//! - `view()` 对权威树按 gitignore 式规则求值（**仅文件行**，目录恒显）。
//!
//! 状态放在 `Arc<Mutex<SourceInner>>` 共享槽：扫描任务在 `&mut self`
//! 之外落树；trait 方法只在主线程做短临界区读写（不跨 await 持锁）。

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use gpui_kit::{App, AsyncApp, Task, WeakEntity, Window};

use crate::app::TerminalApp;
use crate::project_model;
use crate::property_detail_manager::OpenObject;

use super::filter::Ruleset;
use super::{
    MenuDesc, SourceEffect, SourceNode, SourceNodeKind, SourceView, WorkspacePropertySource,
};

/// spec 锚定的默认排除表（file-tree spec: Default scan exclusions）。
pub const EXCLUDES: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "build",
    "dist",
    ".DS_Store",
];

/// 右键功能 id（spec: Right-click context menu）。
pub const FUNC_COPY_PATH: &str = "copy-path";
pub const FUNC_REVEAL: &str = "reveal";

/// 权威树 + 规则的共享槽状态。
#[derive(Default)]
struct SourceInner {
    /// 合成根（id = workspace 根路径；children = 顶层条目）；None = 未就绪。
    canonical: Option<SourceNode>,
    /// 已定位的根；None = 无活跃 workspace。refresh 入口即写入 —
    /// 因此它同时表达"扫描中的根"与"已落地的根" (D6 分轨: 面板不再
    /// 拿 last_synced 视图当当前根, 在途扫描不会被通知重启)。
    root: Option<PathBuf>,
    /// 降级信息（根不可读；落地时写入）。
    degraded: Option<String>,
    /// 当前生效的过滤规则（refresh 时重读）。
    rules: Vec<String>,
    /// 视图代数 (design D5): refresh 入口与扫描落地各 +1 — 输入
    /// (根/规则/权威树) 任一失效即推进; 面板以此做 O(1) 失效检测,
    /// 替代整树深比较 (spec: Frame-driven notifications do not rebuild)。
    generation: u64,
    /// 视图缓存 (design D5): 代数命中 → 直接返回 (零过滤)。
    cached_view: Option<(u64, SourceView)>,
    /// 过滤重建计数 (task 4.1 测试缝: 证明缓存命中不重过滤)。
    #[cfg(test)]
    filter_runs: u64,
}

impl SourceInner {
    fn clear(&mut self) {
        self.canonical = None;
        self.root = None;
        self.degraded = None;
    }

    fn land(&mut self, outcome: Result<SourceNode, String>, root: PathBuf) {
        match outcome {
            Ok(node) => {
                self.canonical = Some(node);
                self.root = Some(root);
                self.degraded = None;
            }
            Err(message) => {
                self.canonical = None;
                self.root = Some(root);
                self.degraded = Some(message);
            }
        }
        // 扫描落地 (design D5 失效源之一) → 视图代数 +1
        self.bump();
    }

    /// 视图代数推进 (缓存失效)。调用点 = refresh 入口 (覆盖根变化/
    /// 规则变化/显式刷新三个失效源) 与 land (扫描落地) — D5 的四个
    /// 失效源全部落在增量点旁。
    fn bump(&mut self) {
        self.generation += 1;
    }

    /// 带代数缓存的视图 (design D5): 代数未变 → 返回缓存副本 (零过滤);
    /// 变化 → 重算一次并回填。`FileTreeSource::view` 委托于此。
    fn view(&mut self) -> SourceView {
        if let Some((generation, cached)) = &self.cached_view {
            if *generation == self.generation {
                return cached.clone();
            }
        }
        #[cfg(test)]
        {
            self.filter_runs += 1;
        }
        let view = self.compute_view();
        self.cached_view = Some((self.generation, view.clone()));
        view
    }

    /// 由权威树 + 规则求值视图 (原 view 主体, 仅代数变化时执行)。
    fn compute_view(&self) -> SourceView {
        if let Some(message) = &self.degraded {
            return SourceView::Degraded {
                root: self.root.clone().expect("degraded implies root"),
                message: message.clone(),
            };
        }
        match (&self.root, &self.canonical) {
            (Some(root), Some(node)) => {
                let ruleset = Ruleset::compile(&self.rules);
                SourceView::Tree {
                    root: root.clone(),
                    node: filter_tree(node, &node.id, &ruleset),
                    rules: self.rules.clone(),
                }
            }
            _ => SourceView::Empty,
        }
    }
}

pub struct FileTreeSource {
    app: WeakEntity<TerminalApp>,
    inner: Arc<Mutex<SourceInner>>,
    /// 刷新任务存根——drop 会取消 future（app.rs 同款约定）；
    /// 新 refresh 覆盖旧任务 = 新扫描取代在途旧扫描（supersede）。
    _refresh_task: Option<Task<()>>,
    /// 后台扫描启动计数 (task 5.1 测试缝): 每次 spawn 扫描 +1。
    #[cfg(test)]
    scan_starts: std::rc::Rc<std::cell::Cell<u64>>,
}

impl FileTreeSource {
    pub fn new(app: WeakEntity<TerminalApp>) -> Self {
        Self {
            app,
            inner: Arc::new(Mutex::new(SourceInner::default())),
            _refresh_task: None,
            #[cfg(test)]
            scan_starts: std::rc::Rc::new(std::cell::Cell::new(0)),
        }
    }

    /// task 5.1 测试缝: 扫描启动计数句柄 (源 Box 化进面板前克隆出来)。
    #[cfg(test)]
    pub(crate) fn scan_starts_handle(&self) -> std::rc::Rc<std::cell::Cell<u64>> {
        self.scan_starts.clone()
    }
}

/// 定位活动 agent 所属 workspace 的根路径（file-tree spec: Tree scope）。
///
/// 单工作区 (change: single-workspace-project-model): 活动会话在当前
/// workspace 内 → 其目录即根；无活动会话 / 未开工作区 → None (空态)。
/// `refresh` / `refresh_if_needed` 用它定位期望根 — 通知路径的
/// "同根才不重扫"比对也收敛在源内 (design D6, 面板不再自行比对)。
pub fn resolve_active_root(app: &WeakEntity<TerminalApp>, cx: &App) -> Option<PathBuf> {
    let app = app.upgrade()?;
    let app = app.read(cx);
    let active = app.active?;
    let ws = app.workspace.as_ref()?;
    ws.agents
        .iter()
        .any(|a| a.entity_id() == active)
        .then(|| ws.path.clone())
}

/// 全量递归扫描（**后台线程执行**；design D6）。
///
/// - 根不可读 → `Err`（面板落降级态，spec: Empty and degraded states）；
/// - 子目录不可读 → 跳过其内容（best-effort，目录仍显示）；
/// - 用 `file_type()`（不跟随符号链接）：链接按文件显示，天然规避
///   符号链接环导致的递归死循环；
/// - 排序：目录在前、文件在后，各自按标签（大小写不敏感）字典序。
pub fn scan_root(root: &Path) -> Result<SourceNode, String> {
    if !root.is_dir() {
        return Err(format!("{} 不是可读目录", root.display()));
    }
    let children = scan_dir(root)?;
    Ok(SourceNode {
        id: root.to_string_lossy().to_string(),
        label: root.display().to_string(),
        kind: SourceNodeKind::Dir,
        children,
    })
}

fn scan_dir(dir: &Path) -> Result<Vec<SourceNode>, String> {
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
            nodes.push(SourceNode {
                id,
                label,
                kind: SourceNodeKind::Dir,
                children,
            });
        } else {
            nodes.push(SourceNode {
                id,
                label,
                kind: SourceNodeKind::File,
                children: Vec::new(),
            });
        }
    }
    nodes.sort_by(|a, b| {
        let rank = |n: &SourceNode| match n.kind {
            SourceNodeKind::Dir => 0,
            SourceNodeKind::File => 1,
        };
        rank(a)
            .cmp(&rank(b))
            .then_with(|| a.label.to_lowercase().cmp(&b.label.to_lowercase()))
            .then_with(|| a.label.cmp(&b.label))
    });
    Ok(nodes)
}

/// 规则求值后的视图树：**目录恒显**（结构保留），文件按 `Ruleset` 过滤。
///
/// `root_id` = 合成根的 id（workspace 根路径），用于剥离出相对路径。
pub fn filter_tree(node: &SourceNode, root_id: &str, rules: &Ruleset) -> SourceNode {
    let rel_of = |id: &str| -> String {
        id.strip_prefix(root_id)
            .and_then(|rest| rest.strip_prefix('/'))
            .map(str::to_string)
            .unwrap_or_else(|| id.to_string())
    };
    let children = node
        .children
        .iter()
        .filter(|child| child.kind == SourceNodeKind::Dir || rules.is_visible(&rel_of(&child.id)))
        .map(|child| filter_tree(child, root_id, rules))
        .collect();
    SourceNode {
        id: node.id.clone(),
        label: node.label.clone(),
        kind: node.kind,
        children,
    }
}

impl WorkspacePropertySource for FileTreeSource {
    fn name(&self) -> &str {
        "文件树"
    }

    fn refresh(&mut self, cx: &mut App) {
        // part1a: 定位根 (活跃 agent 所属 workspace)
        let root = resolve_active_root(&self.app, cx);
        // part1b: 重读规则 — 工作区自己的配置文件 (缺失/损坏 = 空 = 全显;
        // 启动期坏文件契约已由打开路径拦截, 刷新期容错 — file-tree delta)
        let rules = root
            .as_deref()
            .map(project_model::load_file_tree_rules)
            .unwrap_or_default();

        let inner = self.inner.clone();
        let app = self.app.clone();
        {
            let mut guard = inner.lock().expect("source inner lock");
            guard.rules = rules;
            match &root {
                None => guard.clear(), // 无活跃 workspace → 空态 (调用方自行 notify)
                Some(r) => {
                    if guard.root.as_ref() != Some(r) {
                        // 换根: 先清空 (spec: 切换 workspace 替换树; 不闪旧树)
                        guard.canonical = None;
                        guard.degraded = None;
                        guard.root = Some(r.clone());
                    }
                    // 同根 (⟳ 刷新): 保留旧树直到新扫描落地
                }
            }
            // 显式 refresh 入口必使代数 +1 (design D5): 根变化/规则变化/
            // 显式刷新三个失效源都经此; 同根同规则的 ⟳ 也失效 —
            // 用户可见的逃生舱 (Risks: manual refresh always bumps)。
            guard.bump();
        }
        let Some(root) = root else { return };

        // part1c: 后台扫描 → 主线程落树 → 经 app 枢纽转发通知
        // (spec: 面板自读不重建, 必须显式转发; notify_panels 内部 defer)
        let task = cx.spawn(async move |cx: &mut AsyncApp| {
            let outcome = cx
                .background_executor()
                .spawn(async move { scan_root(&root) })
                .await;
            {
                let mut guard = inner.lock().expect("source inner lock");
                let root = guard.root.clone().expect("root set before scan");
                guard.land(outcome, root);
            }
            app.update(cx, |app, cx| app.notify_panels(cx)).ok();
        });
        self._refresh_task = Some(task);
        #[cfg(test)]
        self.scan_starts.set(self.scan_starts.get() + 1);
    }

    fn refresh_if_needed(&mut self, cx: &mut App) {
        // 通知路径 (design D6; spec: In-flight scan is not restarted by
        // notifications): 期望根 == 源当前根 (已落地**或扫描在途** —
        // root 在 refresh 入口即写入) → 无失效事件: 早退 — 不读规则盘、
        // 不取消在途扫描 (spec: notification O(1), no disk reads)。
        // 降级态同根也不自动重试 — 只有 ⟳ / 根再变才重扫。
        let expected = resolve_active_root(&self.app, cx);
        {
            let guard = self.inner.lock().expect("source inner lock");
            if guard.root == expected {
                return;
            }
        }
        // 首载 (None→Some) / 根变化 / 清空 (Some→None) → 强制重扫
        self.refresh(cx);
    }

    fn view_generation(&self) -> u64 {
        self.inner.lock().expect("source inner lock").generation
    }

    fn view(&self) -> SourceView {
        // 代数缓存 (design D5): 未失效 → 直接返回缓存 (零过滤)
        self.inner.lock().expect("source inner lock").view()
    }

    fn menu(&self, _item_id: &str) -> Vec<MenuDesc> {
        // 功能描述由数据源提供 (proposal): v1 两项, 端到端链路可验证
        vec![
            MenuDesc {
                id: FUNC_COPY_PATH.to_string(),
                label: "复制路径".to_string(),
            },
            MenuDesc {
                id: FUNC_REVEAL.to_string(),
                label: "在 Finder 显示".to_string(),
            },
        ]
    }

    fn execute(&mut self, item_id: &str, func_id: &str) -> Vec<SourceEffect> {
        // 执行结果 = 效果意图 (design D2): 落地 (剪贴板/reveal) 由面板完成
        match func_id {
            FUNC_COPY_PATH => vec![SourceEffect::CopyText(item_id.to_string())],
            FUNC_REVEAL => vec![SourceEffect::RevealPath(PathBuf::from(item_id))],
            _ => Vec::new(),
        }
    }

    fn open(&mut self, item_id: &str, window: &mut Window, cx: &mut App) {
        // 查权威树得 kind (tasks 4.1) → 构造中立 OpenObject → app 分发。
        // 树中不存在的 id → no-op; 目录 kind 在 handler 认领阶段被拒,
        // 天然 no-op (spec: Unclaimed object opens nothing)。
        let kind = {
            let guard = self.inner.lock().expect("source inner lock");
            guard
                .canonical
                .as_ref()
                .and_then(|node| super::find_node_kind(node, item_id))
        };
        let Some(kind) = kind else {
            return;
        };
        let obj = OpenObject {
            path: PathBuf::from(item_id),
            kind,
        };
        self.app
            .update(cx, |app, cx| app.open_object(obj, window, cx))
            .ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每测试独立 temp 根 (进程 id + 名字, 并行安全)。
    fn temp_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("filetree-{name}-{}", std::process::id()));
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

    fn labels(nodes: &[SourceNode]) -> Vec<&str> {
        nodes.iter().map(|n| n.label.as_str()).collect()
    }

    fn find<'a>(nodes: &'a [SourceNode], label: &str) -> Option<&'a SourceNode> {
        nodes.iter().find(|n| n.label == label)
    }

    #[test]
    fn filetree_scan_excludes_and_kinds() {
        let root = temp_root("scan");
        // 正常内容
        touch(&root.join("src/app.rs"));
        touch(&root.join("src/pane/mod.rs"));
        touch(&root.join("README.md"));
        touch(&root.join(".gitignore")); // dotfile 必须显示
        // 排除表全量播种 (任意深度)
        for excluded in EXCLUDES {
            touch(&root.join(format!("src/{excluded}/junk.js"))); // 任意深度排除
            touch(&root.join(excluded)); // 文件形态也排除 (.DS_Store 等)
        }
        // 深层 node_modules
        touch(&root.join("src/deep/node_modules/dep/index.js"));

        let tree = scan_root(&root).expect("root 可读");
        assert_eq!(tree.kind, SourceNodeKind::Dir);

        // 顶层: 目录在前 + 排除项缺席 + dotfile 在场
        let top = labels(&tree.children);
        assert_eq!(top, vec!["src", ".gitignore", "README.md"], "顶层排序: 目录前, 大小写不敏感字典序; 排除项全部缺席");

        // 递归无任何排除项
        fn assert_no_excluded(node: &SourceNode) {
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

        // kind 正确
        let src = find(&tree.children, "src").expect("src 在场");
        assert_eq!(src.kind, SourceNodeKind::Dir);
        let app_rs = find(&src.children, "app.rs").expect("app.rs 在场");
        assert_eq!(app_rs.kind, SourceNodeKind::File);
        let pane = find(&src.children, "pane").expect("pane 在场");
        assert_eq!(pane.kind, SourceNodeKind::Dir);
        assert_eq!(labels(&pane.children), vec!["mod.rs"]);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn filetree_scan_missing_root_errors() {
        let missing = std::env::temp_dir().join(format!(
            "filetree-gone-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&missing);
        assert!(scan_root(&missing).is_err(), "根不存在 → Err (降级态输入)");
        // 文件而非目录同样 Err
        let file = std::env::temp_dir().join(format!("filetree-file-{}", std::process::id()));
        std::fs::write(&file, "x").unwrap();
        assert!(scan_root(&file).is_err());
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn view_cache_hits_on_unchanged_generation() {
        // task 4.1 (spec: Unchanged view serves from cache): 代数未变的
        // 两次 view() 第二次走缓存 — filter_runs (过滤路径 spy) 不增。
        let root = temp_root("cache");
        touch(&root.join("src/main.rs"));
        touch(&root.join("notes.log"));
        let tree = scan_root(&root).expect("root 可读");

        let mut inner = SourceInner::default();
        inner.canonical = Some(tree);
        inner.root = Some(root.clone());
        inner.rules = vec!["*.log".to_string()];
        inner.generation = 1;

        // 首取: 计算一次 (规则 *.log 生效)
        let v1 = inner.view();
        assert_eq!(inner.filter_runs, 1, "首 view 必须重过滤");
        let SourceView::Tree { node, .. } = &v1 else {
            panic!("有权威树应得 Tree 视图");
        };
        assert!(find(&node.children, "notes.log").is_none(), "*.log 被过滤");
        assert!(find(&node.children, "src").is_some());

        // 同代数再取: 缓存命中, 不再过滤
        let v2 = inner.view();
        assert_eq!(inner.filter_runs, 1, "同代数必须走缓存 (不重过滤)");
        assert_eq!(v1, v2, "缓存视图与首算视图一致");

        // 代数推进 (扫描落地/规则变化/显式刷新) → 重算一次
        inner.generation += 1;
        let v3 = inner.view();
        assert_eq!(inner.filter_runs, 2, "代数变化必须重过滤");
        assert_eq!(v1, v3, "输入未变时重算结果一致 (非陈旧缓存)");

        // 代数再推进 + 规则变化 → 新过滤结果 (非陈旧)
        inner.generation += 1;
        inner.rules = Vec::new();
        let v4 = inner.view();
        assert_eq!(inner.filter_runs, 3, "规则变化后重过滤");
        let SourceView::Tree { node, .. } = &v4 else {
            panic!("应得 Tree 视图");
        };
        assert!(
            find(&node.children, "notes.log").is_some(),
            "清空规则后文件回来 (缓存不得陈旧)"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn filetree_view_filters_files_only() {
        let root = temp_root("view");
        touch(&root.join("src/main.rs"));
        touch(&root.join("src/main.log"));
        touch(&root.join("docs/readme.md"));
        touch(&root.join("notes.txt"));
        let tree = scan_root(&root).expect("root 可读");
        let root_id = tree.id.clone();

        // ["*.log"] → log 隐藏, 其余与目录全在
        let rules = Ruleset::compile(&["*.log".to_string()]);
        let view = filter_tree(&tree, &root_id, &rules);
        assert_eq!(labels(&view.children), vec!["docs", "src", "notes.txt"]);
        let src = find(&view.children, "src").unwrap();
        assert_eq!(labels(&src.children), vec!["main.rs"], "main.log 被过滤");
        assert!(find(&view.children, "notes.txt").is_some());

        // ["*"] → 文件全隐、目录恒显 (结构保留)
        let all = Ruleset::compile(&["*".to_string()]);
        let view = filter_tree(&tree, &root_id, &all);
        assert_eq!(labels(&view.children), vec!["docs", "src"], "顶层只剩目录");
        let src = find(&view.children, "src").unwrap();
        assert!(src.children.is_empty(), "文件全隐但目录仍在");
        assert_eq!(src.kind, SourceNodeKind::Dir);

        // ["*", "!*.rs"] → 白名单化, 目录结构完整
        let only_rs = Ruleset::compile(&["*".to_string(), "!*.rs".to_string()]);
        let view = filter_tree(&tree, &root_id, &only_rs);
        let src = find(&view.children, "src").unwrap();
        assert_eq!(labels(&src.children), vec!["main.rs"]);
        assert!(find(&view.children, "docs").is_some(), "空内容目录仍显示");

        // 空规则 → 全显
        let empty = Ruleset::compile(&[]);
        let view = filter_tree(&tree, &root_id, &empty);
        assert_eq!(view.children.len(), 3);

        let _ = std::fs::remove_dir_all(&root);
    }
}
