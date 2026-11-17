# Tasks

## 1. asset_manager 骨架与 DTO 迁移 (design D1/D3)

- [ ] 1.1 新建 `src/asset_manager/` 模块并把 DTO 家族改名入驻
  （`AssetNode`/`AssetKind`/`AssetMenu`/`AssetEffect`，id=绝对路径语义
  不变）；`OpenObject.kind` 与 `property_detail_manager` 三处引用换
  `AssetKind` —— 验证: `cargo build` 通过且
  `grep -rn "SourceNode\|MenuDesc\|SourceEffect" src` 只命中 asset_manager
  的迁移注释
- [ ] 1.2 `filter.rs`（Ruleset/glob 语义逐字）平移到 `asset_manager`，
  单测随迁 —— 验证: `cargo test` 中 filter 用例
  （`glob_*` 系列）在新模块路径全绿

## 2. 枚举链 (design D2)

- [ ] 2.1 定义 `AssetEnumeratable`/`AssetEnumerator`（`BoxFuture` 承载异步,
  `Send` 边界）与标准文件枚举器 `FileEnumerator`
  （`scan_root`/排除表/符号链接策略整段平移）；`Project`/`Module` 实现
  `AssetEnumeratable` —— 验证: 单测覆盖 Module root 递归枚举、
  `.git`/`node_modules`/`target`/`build`/`dist`/`.DS_Store` 排除、
  dotfile 保留、dir/file kind 区分（平移自 filetree 扫描测试），`cargo test` 通过
- [ ] 2.2 `AssetManager` Entity：注册表
  （`register(全部)` / `register_project(增量)` / 清空）+ 枚举驱动
  （一次 spawn 内 project→modules→files 组装，**原子 land** + generation+1）
  —— 验证: 无头单测（临时目录）断言注册后 view 形状
  （Project→Module→files 两层行）、land 后 generation 递增、
  `run_until_parked` 前 view 未 land（异步性）

## 3. 过滤与数据侧状态 (design D4/D5)

- [ ] 3.1 过滤挂入 land 路径：逐 Project strip 自己的 root 后跑 Ruleset，
  规则读 `project_model::load_file_tree_rules(ws_dir)` —— 验证: 单测覆盖
  spec 场景（空规则全显、双项目 `*.log` 各自隐藏、`*`+`!*.rs`、
  last-match、`src/**` 相对两项目各自命中），`cargo test` 通过
- [ ] 3.2 manager 数据侧状态与守卫：`Empty`/`Degraded` 三态、refresh
  supersede（覆盖在途任务）、generation 相等 O(1) 早退、数据通知不重启
  在途枚举 —— 验证: 平移旧守卫测试为注册语义
  （在途中通知不重启、显式 refresh 重启、generation 未变零工作、
  模块根删除 → Degraded 且不崩），`cargo test` 通过

## 4. app 层注册接线 (design D6)

- [ ] 4.1 注册钩子：`commit_open` Ready 分支
  （clear + 全量 `ws.model.projects`，紧邻 legacy 规则补拷）、
  `apply_add_solution` 合并后增量注册、右槽 ⟳ 入口改走
  `refresh_assets()` —— 验证: 全链路无头测试：
  打开工作区→项目注册落树；加 solution→新项目行无重启出现；
  切换工作区→旧项目行消失（无残留），`cargo test` 通过

## 5. 右槽面板改写为纯视图 (design D7/D8)

- [ ] 5.1 `tree/mod.rs` 消费 manager view：删 `source: Box<dyn _>` 与
  自管 view 比对，改 generation 相等跳过 + 隐藏期零开销保留；
  `set_source` 测试缝替换为注册缝（测试直接 register 临时项目）——
  验证: 既有面板测试（渲染/展开贴回/渲染缓存/隐藏零开销/重开不重扫）
  改起手式后全绿
- [ ] 5.2 交互件接 manager：⟳ 请求 refresh 且保展开、header 规则摘要读
  `view.rules`、左键经 manager 的 default-open dispatch、右键菜单
  （copy path/reveal）经 manager 执行 —— 验证: 平移测试全过
  （⟳ 保展开、摘要随规则更新、打开计数 stub、菜单效果意图断言），
  `cargo test` 通过
- [ ] 5.3 空/降级显示与门控改注册集（不再依赖活动会话）—— 验证: 无头
  测试：零 project 工作区显示空态提示、无任何活动会话时树照样渲染、
  Degraded 视图显示降级提示且应用存活，`cargo test` 通过

## 6. 删除旧模块与文档 (design D3/迁移计划 step 4)

- [ ] 6.1 删除 `src/workspace_property_source/` 与 `resolve_active_root`，
  `main.rs` 挂载移除 —— 验证:
  `grep -rn "workspace_property_source\|WorkspacePropertySource\|create_source" src`
  零命中，且 `cargo test` 全绿
- [ ] 6.2 更新 `README.md` 架构段（`workspace_property_source` 行 →
  `asset_manager`，右槽描述改 Project 森林）—— 验证: README 架构表与
  实际模块一致，`grep workspace_property_source README.md` 零命中

## 7. 集成验证

- [ ] 7.1 全链路无头回归：注册 → 异步枚举落树 → 过滤 → ⟳ 保展开 →
  加 solution 增量 → 切换清场；验证: `cargo test` 全套通过
- [ ] 7.2 `openspec validate "asset-manager-right-dock" --strict` 通过 +
  `cargo run` 冒烟（打开工作区右树出 Project/Module/文件、⟳、过滤摘要、
  左键开签、右键菜单）

## Workflow follow-up

- 归档 change（`openspec archive`）并同步主 specs。
- 归档后核对主 spec 内容与 git 历史。
