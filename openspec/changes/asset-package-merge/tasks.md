# Tasks

## 1. 归位与改名（机械搬迁，行为冻结）

- [ ] 1.1 建 `src/assets/`：`mod.rs`（模块文档 + 导出伞）、`dto.rs`→`state.rs` 平移并改名 DTO `AssetView`→`AssetState`（含 `find_kind` 形参改 `state`）。验证：`cargo build` 通过；`rg "AssetView::(Empty|Degraded|Forest)"` 全库归零
- [ ] 1.2 trait 定义平移 `src/assets/traits.rs`（旧名暂留，design D6）；`manager.rs`→`src/assets/manager.rs` 并改 `view()`→`state()`，测试外提 `manager_tests.rs`。验证：`cargo build` 通过；`rg "fn view\(" src/assets/` 归零
- [ ] 1.3 `tree/mod.rs`→`src/assets/panel.rs`（`TreePanel`→`AssetView`、`last_synced: Option<AssetState>`），测试外提 `panel_tests.rs`（**只改 use 路径，断言零改写**）。验证：`cargo build` 通过；`rg "TreePanel"` 全库归零
- [ ] 1.4 枚举实现归位 `project_model/enumerate.rs`：`FileEnumerator`/`ModuleEnumerator`/`ProjectEnumerator`、`scan_root`/`scan_dir` 与 impl 块整体平移，`FileFilter` 引用改项目内。验证：`cargo build` 通过；`rg "FileFilter" src/assets/` 归零
- [ ] 1.5 消费方接线：`main.rs`（`mod assets;`）、`shell/mod.rs`（use/构造/`ShellParts.tree`→`asset_view`）、`app.rs`（字段、`notify_panels`、注册点、测试 import）、`property_detail_manager/` 3 处 `AssetKind` import；删除 `src/tree/`、`src/asset_manager/` 残壳。验证：`cargo build` 通过；`rg "asset_manager::|crate::tree|TreePanel"` 全库归零
- [ ] 1.6 组 1 验收：`cargo test --workspace` 全绿且 **268 passed / 0 failed / 0 warnings** 与基线逐数对账；`cargo clippy --all-targets` 干净；新增/迁移函数中文 doc 注释（功能/输入/输出）齐全。验证：命令输出对账 + 组 1 独立 commit

## 2. 边界收紧 Level 2（design D2–D4）

- [ ] 2.1 `traits.rs`：改名 `EnumeratorOwner`/`Enumerator` + `root()` 访问器（中文 doc）；`impl root() for Project` 落 project 域；全量引用更新。验证：`cargo build` 通过；`rg "AssetEnumeratable|AssetEnumerator"` 全库归零
- [ ] 2.2 manager 注册面 trait 化：持 `Vec<Box<dyn EnumeratorOwner + Send>>`；`register`/`register_project` 改收 owner（保 solution 序、身份 = `owner.root()`、重复 root 语义与旧实现逐字一致）；refresh 在主线程逐 owner 调 `enumerator()` 再把 `Box<dyn Enumerator>` 丢 spawn（**owner 不入 spawn**，design D2）；app.rs 注册点跟改。验证：`cargo build` 通过 + 既有 manager 测试全绿（调用点改收 owner）
- [ ] 2.3 `Module.filters` 降私有：注入收敛到 `Project` 构建/`provision_filters` 内部；project_model 内测试改行为断言；`filter.rs` 可见性收紧 `pub(crate)`。验证：`cargo build` 通过；`rg -i "filter" src/assets/` 归零
- [ ] 2.4 manager 测试补假 owner 用例（不改旧用例；新函数中文 doc）。验证：新增测试绿，旧测试数不变
- [ ] 2.5 组 2 验收：`cargo test --workspace` 全绿（268 + 新增）/ 0 warnings 逐数对账；`cargo clippy --all-targets` 干净；asset-manager 10 需求 + file-tree 7 需求映射用例原绿。验证：命令输出对账 + 组 2 独立 commit

## 3. 构造注入（design D7）

- [ ] 3.1 `AssetView::new(manager: Entity<AssetManager>, app: WeakEntity<TerminalApp>, cx)`；`sync_if_generation_changed` 改直读 `self.manager`（`state()`/`view_generation()`）；`right_dock_open` 保留经 app 查 `dock_area`。验证：`cargo build` 通过；panel.rs 内 `\.asset_manager` 字样归零
- [ ] 3.2 `shell::build` 增参传 `&Entity<AssetManager>`；`TerminalApp` 构造顺序不变（manager 先建）。验证：`cargo build` 通过
- [ ] 3.3 `notify_panels` 零改确认（`on_data_changed` 签名不动）。验证：`git diff` 中 `notify_panels` 函数体无改动
- [ ] 3.4 组 3 验收：`cargo test --workspace` 全绿 / 0 warnings 逐数对账；`cargo clippy --all-targets` 干净。验证：命令输出对账 + 组 3 独立 commit

## Workflow follow-up

- 评审满足后 `/opsx-archive` 归档（skip_specs，无 delta 同步）。
- 验证归档结果（`openspec validate --archived`）。
