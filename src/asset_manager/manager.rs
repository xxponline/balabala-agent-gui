//! `AssetManager` — 右槽数据的服务层 (design D1/D5/D6)。
//!
//! 持有注册集 (solution 注册的 Project) 与三态 [`AssetView`] + 代数;
//! 枚举链在一次 spawn 内跑完 (Project → Module → 文件), **原子 land**。
//! 通知纪律: 本实体不直接 notify — 注册由调用方 (app 钩子) 收尾转发,
//! 异步落地经 `app.notify_panels` (内部 defer, 不撞面板 lease)。
//!
//! 守卫 (spec: O(1) / In-flight): 数据变更通知不进入本实体 —
//! 面板只做代数相等比对; 在途任务只被 refresh/注册替换 (supersede)。

#[cfg(test)]
use std::cell::Cell;
use std::path::PathBuf;
#[cfg(test)]
use std::rc::Rc;

use gpui_kit::*;

use crate::app::TerminalApp;
use crate::asset_manager::dto::{AssetEffect, AssetKind, AssetMenu, find_kind};
use crate::project_model::{self, Project};
use crate::property_detail_manager::OpenObject;

use super::dto::AssetView;
use super::enumerator::AssetEnumeratable;

/// 右键功能 id (spec: Right-click context menu)。
pub const FUNC_COPY_PATH: &str = "copy-path";
pub const FUNC_REVEAL: &str = "reveal";

pub struct AssetManager {
    /// 通知枢纽 (落地后经 app.notify_panels 转发)。
    app: WeakEntity<TerminalApp>,
    /// 工作区配置目录 — 规则读取点 (spec: re-read on refresh);
    /// None = 从未注册。
    ws_dir: Option<PathBuf>,
    /// 注册集 (保持 solution 序; 身份 = project root)。
    projects: Vec<Project>,
    /// 三态视图 (design D5)。
    view: AssetView,
    /// 视图代数: 每次失效 (注册/刷新入口) 与每次落地各 +1 —
    /// 面板以相等比对做 O(1) 失效检测 (spec: O(1) when nothing changed)。
    generation: u64,
    /// 在途枚举任务: drop = 取消 (refresh/换注册集即 supersede)。
    in_flight: Option<Task<()>>,
    /// 枚举启动计数 (测试缝: 在途不重启 / 空注册不跑枚举)。
    #[cfg(test)]
    enumerate_starts: Rc<Cell<u64>>,
    /// 视图拉取计数 (测试缝: O(1) 通知守卫)。
    #[cfg(test)]
    view_pulls: Rc<Cell<u64>>,
}

impl AssetManager {
    pub fn new(app: WeakEntity<TerminalApp>, _cx: &mut Context<Self>) -> Self {
        Self {
            app,
            ws_dir: None,
            projects: Vec::new(),
            view: AssetView::Empty,
            generation: 0,
            in_flight: None,
            #[cfg(test)]
            enumerate_starts: Rc::new(Cell::new(0)),
            #[cfg(test)]
            view_pulls: Rc::new(Cell::new(0)),
        }
    }

    /// 拉视图 (面板唯一数据入口)。计数为 O(1) 守卫的测试观测面:
    /// 代数未变的通知不得触发本方法 (spec: … constant work)。
    pub fn view(&self) -> AssetView {
        #[cfg(test)]
        self.view_pulls.set(self.view_pulls.get() + 1);
        self.view.clone()
    }

    /// 视图代数 (面板 O(1) 比对用; spec: Data-change notification is O(1))。
    pub fn view_generation(&self) -> u64 {
        self.generation
    }

    /// 右键菜单 (spec: Right-click context menu — file or directory row;
    /// Project 是分组头, 不在其列; 功能由 manager 提供, 面板落地)。
    pub fn menu(&self, item_id: &str) -> Vec<AssetMenu> {
        match find_kind(&self.view, item_id) {
            Some(kind) if kind.is_menu_row() => vec![
                AssetMenu {
                    id: FUNC_COPY_PATH.to_string(),
                    label: "复制路径".to_string(),
                },
                AssetMenu {
                    id: FUNC_REVEAL.to_string(),
                    label: "在 Finder 显示".to_string(),
                },
            ],
            _ => Vec::new(),
        }
    }

    /// 执行菜单功能 → 效果意图 (面板落地; design D2)。
    /// id 不在视图 / 非菜单行 → no-op。
    pub fn execute(&self, item_id: &str, func_id: &str) -> Vec<AssetEffect> {
        let Some(kind) = find_kind(&self.view, item_id) else {
            return Vec::new(); // 树中不存在 → no-op
        };
        if !kind.is_menu_row() {
            return Vec::new();
        }
        match func_id {
            FUNC_COPY_PATH => vec![AssetEffect::CopyText(item_id.to_string())],
            FUNC_REVEAL => vec![AssetEffect::RevealPath(PathBuf::from(item_id))],
            _ => Vec::new(),
        }
    }

    /// 左键默认打开 (spec: Left-click open hook): 查视图得 kind,
    /// 仅 File 产出 `OpenObject` (目录/模块行由面板切展开, 无打开意图)。
    pub fn open_object_for(&self, item_id: &str) -> Option<OpenObject> {
        match find_kind(&self.view, item_id)? {
            AssetKind::File => Some(OpenObject {
                path: PathBuf::from(item_id),
                kind: AssetKind::File,
            }),
            _ => None,
        }
    }

    /// 全量注册 (workspace 打开/切换; design D6): 替换注册集。
    /// 先清场到 Empty (spec: 切换不闪旧树 / no stale rows) 再异步枚举;
    /// 零注册 → 停在 Empty 且**不跑枚举** (spec: zero-project … no
    /// enumeration runs)。
    pub fn register(
        &mut self,
        ws_dir: PathBuf,
        projects: Vec<Project>,
        cx: &mut Context<Self>,
    ) {
        self.ws_dir = Some(ws_dir);
        self.projects = projects;
        self.in_flight = None; // supersede 旧在途
        self.set_view(AssetView::Empty);
        if self.projects.is_empty() {
            return;
        }
        self.spawn_enumerate(cx);
    }

    /// 增量注册 (add-solution): 保留旧视图直到新枚举落地
    /// (spec: … previous content … until the enumeration lands)。
    pub fn register_project(&mut self, project: Project, cx: &mut Context<Self>) {
        if self.projects.iter().any(|p| p.root == project.root) {
            return; // 幂等: 同 root 不重复注册
        }
        self.projects.push(project);
        if self.ws_dir.is_some() {
            self.spawn_enumerate(cx);
        }
    }

    /// 显式刷新 (⟳): 重读规则 + supersede 在途; 代数入口 +1
    /// (spec: explicit refresh 是缓存失效输入之一)。
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.projects.is_empty() {
            return; // 零注册无物可枚举
        }
        self.generation += 1;
        self.spawn_enumerate(cx);
    }

    /// 立即改视图 (同步路径): 代数 +1。通知由调用方收尾。
    fn set_view(&mut self, view: AssetView) {
        self.view = view;
        self.generation += 1;
    }

    /// 一次 spawn 内跑完整条枚举链, 末尾主线程一次 swap (原子 land)。
    fn spawn_enumerate(&mut self, cx: &mut Context<Self>) {
        let Some(ws_dir) = self.ws_dir.clone() else {
            return; // 从未注册 (defensive)
        };
        let projects = self.projects.clone();
        let app = self.app.clone();
        #[cfg(test)]
        self.enumerate_starts.set(self.enumerate_starts.get() + 1);

        let task = cx.spawn(async move |this, cx: &mut AsyncApp| {
            let bg = cx.background_executor();
            let outcome = bg.spawn({
                let bg = bg.clone();
                async move {
                    // 规则读取: 容错 = 空 (启动期坏文件契约在打开路径拦截)
                    let rules = project_model::load_file_tree_rules(&ws_dir);
                    let mut raw = Vec::with_capacity(projects.len());
                    for project in &projects {
                        let node = project.enumerator().enumerate(bg.clone()).await?;
                        raw.push(node);
                    }
                    // 过滤挂入 land (task 3.1): 逐 Project strip 自己的 root
                    // 后跑 Ruleset — 目录类行恒显, 文件按相对路径求值
                    // (spec: File-row filtering relative to project roots)。
                    let ruleset = crate::asset_manager::filter::Ruleset::compile(&rules);
                    let nodes = raw
                        .into_iter()
                        .zip(projects.iter())
                        .map(|(node, project)| {
                            let root = project.root.to_string_lossy();
                            crate::asset_manager::filter::filter_assets(
                                &node,
                                &root,
                                &ruleset,
                            )
                        })
                        .collect();
                    Ok::<(Vec<String>, Vec<super::dto::AssetNode>), String>((rules, nodes))
                }
            })
            .await;
            // 回主线程: 换 view + 代数 +1, 再经 app 枢纽转发通知
            this.update(cx, |m, _cx| {
                m.view = match outcome {
                    Ok((rules, nodes)) => AssetView::Forest { rules, nodes },
                    Err(message) => AssetView::Degraded { message },
                };
                m.generation += 1;
            })
            .ok();
            app.update(cx, |app, cx| app.notify_panels(cx)).ok();
        });
        self.in_flight = Some(task);
    }

    /// 枚举启动计数 (测试缝)。
    #[cfg(test)]
    pub(crate) fn enumerate_starts_handle(&self) -> Rc<Cell<u64>> {
        self.enumerate_starts.clone()
    }

    /// 视图拉取计数 (测试缝: O(1) 守卫 — 代数未变的通知不得拉视图)。
    #[cfg(test)]
    pub(crate) fn view_pulls_handle(&self) -> Rc<Cell<u64>> {
        self.view_pulls.clone()
    }
}

#[cfg(test)]
mod tests {
    //! manager 的无头契约: 注册 → 异步枚举 → 原子 land → 三态/代数。
    //!
    //! 显式导入, 不用 `super::*`/`gpui_kit::*` glob — 防属性宏展开的内层
    //! `#[test]` 解析到 `gpui_kit::test` → 递归爆 (app.rs/tree 同款教训)。

    use super::{AssetManager, FUNC_COPY_PATH, FUNC_REVEAL};
    use crate::app::TerminalApp;
    use crate::asset_manager::{AssetKind, AssetNode, AssetView, project_id};
    use crate::project_model::{self, Project};
    use gpui_kit::{
        AppContext, Bounds, Point, TestAppContext, WindowBounds, WindowOptions, px, size,
    };
    use std::path::{Path, PathBuf};

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "am-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, "x").unwrap();
    }

    /// 起带空历史的完整 app (manager 挂在其上; 落地通知链可真实跑通)。
    fn open_app(cx: &mut TestAppContext) -> gpui_kit::Entity<TerminalApp> {
        cx.update(gpui_kit::init);
        let (_handle, app) = cx.update(|cx| {
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: Point::default(),
                        size: size(px(1000.), px(600.)),
                    })),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    let hist = std::env::temp_dir().join(format!(
                        "am-hist-{}-{}.json",
                        std::process::id(),
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_nanos()
                    ));
                    let _ = std::fs::remove_file(&hist);
                    cx.new(|cx| {
                        TerminalApp::new_inner(window, cx, hist).expect("new_inner 启动")
                    })
                },
            )
            .expect("open test window")
        });
        // hermetic: 清空可能的恢复态
        app.update(cx, |app, _| {
            app.workspace = None;
            app.active = None;
        });
        app
    }

    fn manager_of(
        cx: &mut TestAppContext,
        app: &gpui_kit::Entity<TerminalApp>,
    ) -> gpui_kit::Entity<AssetManager> {
        cx.update(|cx| app.read(cx).asset_manager.clone())
    }

    /// 2.2 核心: 注册 → 落地前 Empty (异步性) → 落地后两层行 + 代数推进。
    #[gpui_kit::test]
    fn register_lands_forest_atomically(cx: &mut TestAppContext) {
        let app = open_app(cx);
        let manager = manager_of(cx, &app);

        let ws = temp_dir("ws");
        let project_dir = ws.join("foo");
        std::fs::create_dir_all(&project_dir).unwrap();
        touch(&project_dir.join("main.rs"));
        let project = Project::from_root(project_dir.clone());

        let (gen_before, starts) = cx.update(|cx| {
            let m = manager.read(cx);
            (m.view_generation(), m.enumerate_starts_handle().get())
        });
        assert_eq!(gen_before, 0);
        assert_eq!(starts, 0);

        manager.update(cx, |m, cx| m.register(ws.clone(), vec![project], cx));
        // 注册即清场: 落地前视图 Empty (异步性 + 不闪旧树), 枚举已启动
        cx.update(|cx| {
            let m = manager.read(cx);
            assert_eq!(m.view(), AssetView::Empty, "落地前 = 清场后的 Empty");
            assert_eq!(m.enumerate_starts_handle().get(), starts + 1);
        });

        cx.run_until_parked(); // 驱动后台枚举 → 原子落地
        cx.update(|cx| {
            let m = manager.read(cx);
            let view = m.view();
            let AssetView::Forest { rules, nodes } = &view else {
                panic!("应落地 Forest, got {view:?}")
            };
            assert!(rules.is_empty(), "无规则区段 = 空规则");
            assert_eq!(nodes.len(), 1, "一个注册 project 一行");
            let p = &nodes[0];
            assert_eq!(p.kind, AssetKind::Project);
            assert_eq!(p.id, project_id(&project_dir));
            assert_eq!(p.children.len(), 1, "Project → Module 两层行");
            let mrow = &p.children[0];
            assert_eq!(mrow.kind, AssetKind::Module);
            assert_eq!(mrow.id, project_dir.to_string_lossy());
            assert!(
                mrow.children.iter().any(|n| n.label == "main.rs"),
                "Module 下挂扫描出的文件"
            );
            assert!(
                m.view_generation() > gen_before,
                "land 必须推进代数 (面板失效检测)"
            );
        });
    }

    /// 2.2: 零注册 → Empty 且不跑枚举 (spec: zero-project … no enumeration)。
    #[gpui_kit::test]
    fn zero_projects_registers_empty_without_enumerating(cx: &mut TestAppContext) {
        let app = open_app(cx);
        let manager = manager_of(cx, &app);
        let ws = temp_dir("empty-ws");

        manager.update(cx, |m, cx| m.register(ws, Vec::new(), cx));
        cx.run_until_parked();
        cx.update(|cx| {
            let m = manager.read(cx);
            assert_eq!(m.view(), AssetView::Empty);
            assert_eq!(
                m.enumerate_starts_handle().get(),
                0,
                "零注册不得启动枚举"
            );
            assert!(m.view_generation() >= 1, "清场仍推进代数 (面板可同步)");
        });
        // 刷新对零注册也是 no-op (spec: no enumeration runs)
        manager.update(cx, |m, cx| m.refresh(cx));
        cx.run_until_parked();
        cx.update(|cx| {
            assert_eq!(manager.read(cx).enumerate_starts_handle().get(), 0);
            assert_eq!(manager.read(cx).view(), AssetView::Empty);
        });
    }

    /// 模块根不可读 → Degraded 且不崩
    /// (spec: Deleted module root degrades gracefully)。
    #[gpui_kit::test]
    fn missing_module_root_lands_degraded(cx: &mut TestAppContext) {
        let app = open_app(cx);
        let manager = manager_of(cx, &app);
        let ws = temp_dir("degraded-ws");
        let ghost = ws.join("ghost"); // 不创建
        let project = Project::from_root(ghost.clone());

        manager.update(cx, |m, cx| m.register(ws, vec![project], cx));
        cx.run_until_parked();
        cx.update(|cx| {
            let m = manager.read(cx);
            let view = m.view();
            let AssetView::Degraded { message } = &view else {
                panic!("应降级, got {view:?}")
            };
            assert!(
                message.contains("ghost") || message.contains("不是可读目录"),
                "降级要命名失败: {message}"
            );
        });
    }

    /// spec: Hide matching files in every project — 规则从工作区 marker
    /// 读入, land 时逐 Project 各自根求值。
    #[gpui_kit::test]
    fn filtering_applies_to_every_project_at_land(cx: &mut TestAppContext) {
        let app = open_app(cx);
        let manager = manager_of(cx, &app);
        let ws = temp_dir("filter-ws");
        project_model::set_file_tree_rules(&ws, vec!["*.log".to_string()]).unwrap();
        let p1 = ws.join("one");
        let p2 = ws.join("two");
        touch(&p1.join("a.log"));
        touch(&p1.join("b.rs"));
        touch(&p2.join("a.log"));
        touch(&p2.join("b.rs"));
        touch(&p1.join("src/x.log"));
        touch(&p2.join("src/x.log"));
        let projects = vec![
            Project::from_root(p1.clone()),
            Project::from_root(p2.clone()),
        ];

        manager.update(cx, |m, cx| m.register(ws, projects, cx));
        cx.run_until_parked();
        cx.update(|cx| {
            let view = manager.read(cx).view();
            let AssetView::Forest { rules, nodes } = &view else {
                panic!("应落地 Forest, got {view:?}")
            };
            assert_eq!(rules, &vec!["*.log".to_string()], "规则随视图携带");
            assert_eq!(nodes.len(), 2, "两个 project 都在");
            for node in nodes {
                let module = &node.children[0];
                let labels: Vec<&str> =
                    module.children.iter().map(|n| n.label.as_str()).collect();
                assert!(
                    !labels.contains(&"a.log"),
                    "{}: *.log 应隐藏, 实际: {labels:?}",
                    node.label
                );
                assert!(labels.contains(&"b.rs"), "非匹配文件保留");
                let src = module
                    .children
                    .iter()
                    .find(|n| n.label == "src")
                    .expect("目录行恒显");
                assert!(
                    src.children.is_empty(),
                    "目录恒显但其下 *.log 已过滤"
                );
            }
        });
    }

    /// spec: In-flight enumeration is not restarted by notifications —
    /// 数据变更通知 (terminal frame → notify_panels) 不得触碰在途枚举。
    #[gpui_kit::test]
    fn data_notifications_do_not_restart_inflight(cx: &mut TestAppContext) {
        let app = open_app(cx);
        let manager = manager_of(cx, &app);
        let ws = temp_dir("inflight");
        let project_dir = ws.join("p");
        touch(&project_dir.join("a.rs"));
        let project = Project::from_root(project_dir);

        manager.update(cx, |m, cx| m.register(ws, vec![project], cx));
        // 在途期间连发数据变更通知 (面板路径, 不经 manager)
        for _ in 0..3 {
            app.update(cx, |app, cx| app.notify_panels(cx));
        }
        cx.run_until_parked();
        cx.update(|cx| {
            let m = manager.read(cx);
            assert_eq!(
                m.enumerate_starts_handle().get(),
                1,
                "通知不得重启/叠加在途枚举"
            );
            assert!(matches!(m.view(), AssetView::Forest { .. }), "原枚举完成落地");
        });
    }

    /// spec: Explicit refresh still restarts — ⟳ 入口 supersede 在途任务。
    #[gpui_kit::test]
    fn explicit_refresh_restarts_inflight_enumeration(cx: &mut TestAppContext) {
        let app = open_app(cx);
        let manager = manager_of(cx, &app);
        let ws = temp_dir("refresh-restart");
        let project_dir = ws.join("p");
        touch(&project_dir.join("a.rs"));
        let project = Project::from_root(project_dir);

        manager.update(cx, |m, cx| m.register(ws, vec![project], cx));
        // 不 park: 在途中显式刷新
        manager.update(cx, |m, cx| m.refresh(cx));
        let starts_after_refresh =
            cx.update(|cx| manager.read(cx).enumerate_starts_handle().get());
        assert_eq!(
            starts_after_refresh, 2,
            "refresh = 新启一轮 (旧任务 drop 取消)"
        );
        cx.run_until_parked();
        cx.update(|cx| {
            let m = manager.read(cx);
            assert_eq!(m.enumerate_starts_handle().get(), 2, "共两轮");
            assert!(matches!(m.view(), AssetView::Forest { .. }), "最终落地一次");
        });
    }

    /// spec: Data-change notification is O(1) when nothing changed —
    /// 观测代理: 通知后代数不动、视图相等、无新枚举 (零遍历/零读盘的外部
    /// 可见证据)。
    #[gpui_kit::test]
    fn notification_leaves_generation_and_view_unchanged(cx: &mut TestAppContext) {
        let app = open_app(cx);
        let manager = manager_of(cx, &app);
        let ws = temp_dir("o1");
        touch(&ws.join("p/a.rs"));
        let project = Project::from_root(ws.join("p"));
        manager.update(cx, |m, cx| m.register(ws, vec![project], cx));
        cx.run_until_parked();

        let (gen0, view0) = cx.update(|cx| {
            let m = manager.read(cx);
            (m.view_generation(), m.view())
        });
        for _ in 0..5 {
            app.update(cx, |app, cx| app.notify_panels(cx));
        }
        cx.run_until_parked();
        let (gen1, view1, starts) = cx.update(|cx| {
            let m = manager.read(cx);
            (m.view_generation(), m.view(), m.enumerate_starts_handle().get())
        });
        assert_eq!(gen1, gen0, "无失效事件 → 代数不动");
        assert_eq!(view1, view0, "视图相等 → 面板直接命中缓存");
        assert_eq!(starts, 1, "零新枚举");
    }

    /// 5.2 (spec: Right-click context menu / Left-click open hook):
    /// 菜单描述、执行效果映射、打开意图的 kind 门禁 —
    /// 旧 filetree execute/open 断言的继任 (合成视图, 无需磁盘)。
    #[gpui_kit::test]
    fn hooks_map_menu_execute_and_open(cx: &mut TestAppContext) {
        use crate::asset_manager::AssetEffect;
        use crate::property_detail_manager::OpenObject;

        let app = open_app(cx);
        let manager = manager_of(cx, &app);
        // 合成 fixture: Project → Module → (dir, file)
        manager.update(cx, |m, _cx| {
            m.view = AssetView::Forest {
                rules: Vec::new(),
                nodes: vec![AssetNode {
                    id: project_id(Path::new("/a/proj")),
                    label: "proj".to_string(),
                    kind: AssetKind::Project,
                    children: vec![AssetNode {
                        id: "/a/proj".to_string(),
                        label: "proj".to_string(),
                        kind: AssetKind::Module,
                        children: vec![
                            AssetNode {
                                id: "/a/proj/src".to_string(),
                                label: "src".to_string(),
                                kind: AssetKind::Dir,
                                children: vec![],
                            },
                            AssetNode {
                                id: "/a/proj/src/app.rs".to_string(),
                                label: "app.rs".to_string(),
                                kind: AssetKind::File,
                                children: vec![],
                            },
                        ],
                    }],
                }],
            };
        });

        cx.update(|cx| {
            let m = manager.read(cx);
            // 菜单: file/dir 行两项; Project 分组头与未知 id 无菜单
            let file_menu = m.menu("/a/proj/src/app.rs");
            assert_eq!(
                file_menu.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
                vec![FUNC_COPY_PATH, FUNC_REVEAL]
            );
            assert_eq!(m.menu("/a/proj/src").len(), 2, "目录行有菜单");
            assert!(m.menu(&project_id(Path::new("/a/proj"))).is_empty(), "Project 分组头无菜单");
            assert!(m.menu("/nope").is_empty(), "未知 id 无菜单");

            // 效果映射 (旧断言原文): reveal → RevealPath, copy → CopyText
            assert_eq!(
                m.execute("/a/proj/src/app.rs", FUNC_REVEAL),
                vec![AssetEffect::RevealPath(PathBuf::from("/a/proj/src/app.rs"))],
                "在 Finder 显示 → RevealPath 效果"
            );
            assert_eq!(
                m.execute("/a/proj/src/app.rs", FUNC_COPY_PATH),
                vec![AssetEffect::CopyText("/a/proj/src/app.rs".to_string())],
                "复制路径 → CopyText 效果"
            );
            assert!(
                m.execute("/a/proj/src/app.rs", "unknown-func").is_empty(),
                "未知功能 → 无效果"
            );
            assert!(m.execute("/nope", FUNC_COPY_PATH).is_empty(), "不存在的 id → no-op");

            // 打开意图: 仅 File 产出 OpenObject (kind 门禁)
            assert_eq!(
                m.open_object_for("/a/proj/src/app.rs"),
                Some(OpenObject {
                    path: PathBuf::from("/a/proj/src/app.rs"),
                    kind: AssetKind::File,
                })
            );
            assert_eq!(m.open_object_for("/a/proj/src"), None, "目录行无打开意图");
            assert_eq!(m.open_object_for("/a/proj"), None, "模块行无打开意图");
            assert_eq!(m.open_object_for("/nope"), None, "未知 id 无意图");
        });
    }
}
