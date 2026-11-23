    //! manager 的无头契约: 注册 → 异步枚举 → 原子 land → 三态/代数。
    //!
    //! 显式导入, 不用 `super::*`/`gpui_kit::*` glob — 防属性宏展开的内层
    //! `#[test]` 解析到 `gpui_kit::test` → 递归爆 (app.rs/tree 同款教训)。

    use super::{AssetManager, FUNC_COPY_PATH, FUNC_REVEAL};
    use crate::app::TerminalApp;
    use crate::assets::state::{AssetKind, AssetNode, AssetState, project_id};
    use crate::assets::traits::{BoxFuture, Enumerator, EnumeratorOwner};
    use crate::project_model::Project;
    use gpui_kit::{
        AppContext, BackgroundExecutor, Bounds, Point, TestAppContext, WindowBounds,
        WindowOptions, px, size,
    };
    use std::path::{Path, PathBuf};

    /// Level 2 注册面装箱 (测试夹具): Project -> owner。
    fn own(p: Project) -> Box<dyn EnumeratorOwner + Send> {
        Box::new(p)
    }

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

        manager.update(cx, |m, cx| m.register(ws.clone(), vec![own(project)], cx));
        // 注册即清场: 落地前视图 Empty (异步性 + 不闪旧树), 枚举已启动
        cx.update(|cx| {
            let m = manager.read(cx);
            assert_eq!(m.state(), AssetState::Empty, "落地前 = 清场后的 Empty");
            assert_eq!(m.enumerate_starts_handle().get(), starts + 1);
        });

        cx.run_until_parked(); // 驱动后台枚举 → 原子落地
        cx.update(|cx| {
            let m = manager.read(cx);
            let view = m.state();
            let AssetState::Forest { nodes } = &view else {
                panic!("应落地 Forest, got {view:?}")
            };
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
            assert_eq!(m.state(), AssetState::Empty);
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
            assert_eq!(manager.read(cx).state(), AssetState::Empty);
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

        manager.update(cx, |m, cx| m.register(ws, vec![own(project)], cx));
        cx.run_until_parked();
        cx.update(|cx| {
            let m = manager.read(cx);
            let view = m.state();
            let AssetState::Degraded { message } = &view else {
                panic!("应降级, got {view:?}")
            };
            assert!(
                message.contains("ghost") || message.contains("不是可读目录"),
                "降级要命名失败: {message}"
            );
        });
    }

    /// spec: Hide matching files in every project — 规则来自各项目根
    /// `.gitignore` (供给链: git 判定 + 读取 + 注入), land 时逐
    /// Project 各自根求值。
    #[gpui_kit::test]
    fn per_project_rules_apply_at_land(cx: &mut TestAppContext) {
        let app = open_app(cx);
        let manager = manager_of(cx, &app);
        let ws = temp_dir("rules-ws");
        let p1 = ws.join("one");
        let p2 = ws.join("two");
        // 供给前提: 两个项目各自 git 管理 + 项目根 .gitignore 隐藏 *.log
        for p in [&p1, &p2] {
            std::fs::create_dir_all(p.join(".git")).unwrap();
            std::fs::write(p.join(".gitignore"), "*.log\n").unwrap();
        }
        touch(&p1.join("a.log"));
        touch(&p1.join("b.rs"));
        touch(&p2.join("a.log"));
        touch(&p2.join("b.rs"));
        touch(&p1.join("src/x.log"));
        touch(&p2.join("src/x.log"));
        let projects = vec![
            // 供给前提: 项目 git 管理 → is_git 解析产出 = true (change
            // solution-service 4.2: 判定来自标志, 不再运行时探测)
            Project::from_root_with_git(p1.clone(), true),
            Project::from_root_with_git(p2.clone(), true),
        ]
        .into_iter()
        .map(own)
        .collect();

        manager.update(cx, |m, cx| m.register(ws, projects, cx));
        cx.run_until_parked();
        cx.update(|cx| {
            let view = manager.read(cx).state();
            let AssetState::Forest { nodes } = &view else {
                panic!("应落地 Forest, got {view:?}")
            };
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

    /// spec: Dot entries absent at any depth + Dependency directories
    /// present by default — dot-only 默认经全链路 (注册 → 枚举 → land)
    /// 生效; 非 git 项目不读任何 ignore 文件 (design D8)。
    #[gpui_kit::test]
    fn dot_only_default_applies_through_full_chain(cx: &mut TestAppContext) {
        let app = open_app(cx);
        let manager = manager_of(cx, &app);
        let ws = temp_dir("excl-ws");
        let p = ws.join("proj");
        touch(&p.join("src/main.rs"));
        touch(&p.join("node_modules/dep/index.js"));
        touch(&p.join(".git/config"));
        touch(&p.join(".gitignore")); // dot 缺席, 且非 git 项目本就不读
        touch(&p.join("target/debug/bin"));
        let project = Project::from_root(p.clone());

        manager.update(cx, |m, cx| m.register(ws, vec![own(project)], cx));
        cx.run_until_parked();
        cx.update(|cx| {
            let view = manager.read(cx).state();
            let AssetState::Forest { nodes, .. } = &view else {
                panic!("应落地 Forest, got {view:?}")
            };
            assert_eq!(nodes.len(), 1, "一个 project");

            fn labels_in<'a>(node: &'a AssetNode, out: &mut Vec<&'a str>) {
                out.push(&node.label);
                for child in &node.children {
                    labels_in(child, out);
                }
            }
            let mut labels = Vec::new();
            for node in nodes {
                labels_in(node, &mut labels);
            }
            for label in labels.iter() {
                assert!(!label.starts_with('.'), "dot 条目泄漏: {labels:?}");
            }
            assert!(
                labels.contains(&"node_modules"),
                "六项撤销: 依赖目录默认在场"
            );
            assert!(labels.contains(&"target"), "六项撤销: 构建目录默认在场");
            assert!(labels.contains(&"src"), "正常目录保留");
            assert!(labels.contains(&"main.rs"), "正常文件保留");
        });
    }

    /// spec: Workspace-owned files are not managed — marker/solutions 在
    /// 项目根之外的 workspace 目录, 永不入树 (且 dot 名被默认排除双重
    /// 把守)。
    #[gpui_kit::test]
    fn workspace_owned_files_never_in_view(cx: &mut TestAppContext) {
        let app = open_app(cx);
        let manager = manager_of(cx, &app);
        let ws = temp_dir("owned-ws");
        crate::project_model::create_workspace_files(&ws).unwrap(); // marker + solutions 在 ws 根
        let p = ws.join("proj");
        touch(&p.join("main.rs"));
        let project = Project::from_root(p);

        manager.update(cx, |m, cx| m.register(ws, vec![own(project)], cx));
        cx.run_until_parked();
        cx.update(|cx| {
            let view = manager.read(cx).state();
            let AssetState::Forest { nodes } = &view else {
                panic!("应落地 Forest, got {view:?}")
            };
            fn labels_in<'a>(node: &'a AssetNode, out: &mut Vec<&'a str>) {
                out.push(&node.label);
                for child in &node.children {
                    labels_in(child, out);
                }
            }
            let mut labels = Vec::new();
            for node in nodes {
                labels_in(node, &mut labels);
            }
            assert!(
                !labels.contains(&".balabala.workspace.json")
                    && !labels.contains(&"balabala.solutions.json"),
                "workspace 自有文件永不入树: {labels:?}"
            );
            assert!(labels.contains(&"main.rs"), "项目文件照常");
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

        manager.update(cx, |m, cx| m.register(ws, vec![own(project)], cx));
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
            assert!(matches!(m.state(), AssetState::Forest { .. }), "原枚举完成落地");
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

        manager.update(cx, |m, cx| m.register(ws, vec![own(project)], cx));
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
            assert!(matches!(m.state(), AssetState::Forest { .. }), "最终落地一次");
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
        manager.update(cx, |m, cx| m.register(ws, vec![own(project)], cx));
        cx.run_until_parked();

        let (gen0, view0) = cx.update(|cx| {
            let m = manager.read(cx);
            (m.view_generation(), m.state())
        });
        for _ in 0..5 {
            app.update(cx, |app, cx| app.notify_panels(cx));
        }
        cx.run_until_parked();
        let (gen1, view1, starts) = cx.update(|cx| {
            let m = manager.read(cx);
            (m.view_generation(), m.state(), m.enumerate_starts_handle().get())
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
        use crate::assets::AssetEffect;
        use crate::property_detail_manager::OpenObject;

        let app = open_app(cx);
        let manager = manager_of(cx, &app);
        // 合成 fixture: Project → Module → (dir, file)
        manager.update(cx, |m, _cx| {
            m.view = AssetState::Forest {
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


    /// 假资产源 (Level 2 测试缝, design D2): 不碰文件系统, 直接产出
    /// 固定子树 — manager 只见 `EnumeratorOwner` 契约, 无需真实 Project。
    struct FakeOwner {
        /// 身份根 (增量注册幂等判据)。
        root: PathBuf,
        /// 固定行 (枚举直接产出)。
        node: AssetNode,
    }

    impl EnumeratorOwner for FakeOwner {
        fn root(&self) -> &Path {
            &self.root
        }

        fn enumerator(&self) -> Box<dyn Enumerator> {
            // 快照在主线程 (owner 不入 spawn 的契约同款)
            let node = self.node.clone();
            Box::new(FakeEnumerator { node })
        }
    }

    /// 假枚举执行体: 直接返回固定节点 (无后台 IO)。
    struct FakeEnumerator {
        node: AssetNode,
    }

    impl Enumerator for FakeEnumerator {
        fn enumerate(
            self: Box<Self>,
            _bg: BackgroundExecutor,
        ) -> BoxFuture<'static, Result<AssetNode, String>> {
            let node = self.node;
            Box::pin(async move { Ok(node) })
        }
    }

    /// 构造假源: 身份根 = `root`, 单行 id/label 由 `label` 派生。
    fn fake(root: &Path, label: &str) -> FakeOwner {
        FakeOwner {
            root: root.to_path_buf(),
            node: AssetNode {
                id: root.join(label).display().to_string(),
                label: label.to_string(),
                kind: AssetKind::Dir,
                children: Vec::new(),
            },
        }
    }

    /// 2.4: 假源批量注册 — 保 solution 序落地, 全程不触文件系统。
    #[gpui_kit::test]
    fn fake_owners_land_in_order_without_fs(cx: &mut TestAppContext) {
        let app = open_app(cx);
        let manager = manager_of(cx, &app);
        let ws = temp_dir("fake-ws");

        manager.update(cx, |m, cx| {
            m.register(
                ws,
                vec![
                    Box::new(fake(Path::new("/fake-a"), "alpha"))
                        as Box<dyn EnumeratorOwner + Send>,
                    Box::new(fake(Path::new("/fake-b"), "beta"))
                        as Box<dyn EnumeratorOwner + Send>,
                ],
                cx,
            );
        });
        cx.run_until_parked();
        cx.update(|cx| {
            let view = manager.read(cx).state();
            let AssetState::Forest { nodes } = &view else {
                panic!("应落地 Forest, got {view:?}")
            };
            let labels: Vec<&str> = nodes.iter().map(|n| n.label.as_str()).collect();
            assert_eq!(labels, ["alpha", "beta"], "保 solution 序");
        });
    }
