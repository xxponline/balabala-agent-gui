# Tasks

## 1. 数据源地基（workspace_property_source）

- [x] 1.1 建 `src/workspace_property_source/{mod.rs,filetree.rs}`：`WorkspacePropertySource` trait（`name/refresh/view/menu/execute/open`，dyn-safe）+ 中立 DTO（`SourceNode`/`MenuDesc`/`SourceEffect`）+ 静态注册表；验证 `cargo check` 通过
- [x] 1.2 glob 过滤引擎（design D5）：`*`（不跨 `/`）/`?`/`**`/路径模式、后写覆盖、`!` 反悔，仅作用于文件行；验证：单测覆盖空规则全显、`["*.log"]` 隐藏、`["*","!*.rs"]` 白名单化、last-wins、路径模式（`cargo test glob_`）
- [x] 1.3 `store.rs` 扩展：`Persisted` 增 `file_tree` 区段（serde default、空省略）+ `load_file_tree_rules(path)` + `save` 改为 load→改 workspaces→写回（保全区段）；验证：单测 roundtrip 保全 `file_tree`、旧格式文件照常解析（`cargo test store_`）
- [x] 1.4 `FileTreeSource`：`refresh()` 经 `WeakEntity<TerminalApp>` 定位 active ws 根 + 读规则 + 后台扫描（排除表）落权威树；`view()` 规则求值；`menu()` 两项（复制路径/在 Finder 显示）→ `execute()` 返回 `CopyText`/`RevealPath` 效果；`open()` v1 no-op；验证：tempdir 单测扫描排除（`.git node_modules target build dist .DS_Store` 缺席、dotfile 在场、Dir/File kind 正确）+ view 集成过滤（`cargo test filetree_`）

## 2. 面板骨架与装配

- [x] 2.1 `src/tree/mod.rs` 实现 `TreePanel`（`Panel`：`title_bar=false`、`inner_padding=false`）：header（`source.name()` 标题 + ⟳ + 只读规则摘要 + `×`→`close_right_dock`）+ body（空态占位）；`shell/mod.rs` 以 `panel_handle` 换装、删除 `shell/placeholder.rs`；验证 `cargo test` 既有 14+1 全绿，`cargo run`（headed）右槽显示新面板、`×` 关闭与拖拽尺寸正常
- [x] 2.2 源注入缝：面板构造从注册表取 `Box<dyn WorkspacePropertySource>`（默认 filetree），提供测试构造器注入 stub 源；验证：headless 测试 `panel_renders_stub_source`（stub 树出现在右槽）

## 3. 树渲染与范围

- [x] 3.1 适配器：`SourceNode→TreeItem`（按面板自持 expanded 集合贴 `.expanded()`，`dir_ids` 集合修正空目录图标）+ `TreeState` 接线 + `TreeEvent::Expanded/Collapsed` 维护集合；验证：headless `tree_adapter_roundtrip`（渲染根节点；展开/折叠后集合与视图一致）
- [x] 3.2 active 范围接线：`app.rs` 把树面板加入 `notify_panels` 转发名单（沿用 `cx.defer` 防重入），重扫粒度 = 根路径变化；验证：headless `tree_scope_follows_active_workspace`（切 workspace 换树；同 ws 切 agent 不重扫）+ `no_active_agent_shows_empty_state`

## 4. 交互

- [x] 4.1 左键：`render_item` 的 `ListItem.on_click` → `source.open(id)`（stub 计数）+ 焦点交回终端面板；验证：headless `file_click_invokes_open_once`、`tree_click_keeps_keyboard_with_session`（焦点 handle 断言），并断言库内置"选中/目录切换"与 open 分发共存恰各一次（spike 项落测）
- [x] 4.2 右键：`Tree::context_menu` 按 index→entry→id 取 `source.menu(id)` 构建 `PopupMenu`；项点击 → `source.execute` → 面板执行效果（`CopyText`→`window.write_to_clipboard`；`RevealPath`→`open -R`，`cfg(target_os="macos")`）；验证：headless `right_click_shows_source_menu`、`copy_path_writes_clipboard`；reveal 效果产出断言 + headed 手动确认 Finder 定位

## 5. 刷新与过滤配置

- [x] 5.1 刷新触发：首次加载 / 根路径变化 / ⟳ 点击，扫描走后台执行器；验证：headless `refresh_picks_up_new_files`（tempdir 建文件→⟳→树中出现）
- [x] 5.2 规则重读：refresh 重读 `file_tree` 区段（hermetic：`rules_path` 测试缝指向临时文件）；验证：headless `refresh_rereads_filter_rules`（改规则文件→⟳→过滤变化）
- [x] 5.3 展开态跨刷新保留，刷新后对新树裁剪 expanded 集合；验证：headless `refresh_preserves_expansion`
- [x] 5.4 降级态：根路径缺失/不可读时刷新 → 降级提示、不崩、不把旧内容当现状；验证：headless `deleted_root_degrades_gracefully`
- [x] 5.5 header 只读规则摘要随规则更新；验证：headless 断言摘要文本（`header_shows_rule_summary`）

## 6. 集成验收

- [x] 6.1 全量回归：`cargo test` 全绿（既有 14+1 + 本变更新增）、`cargo build` 0 warning、`openspec validate --all` 通过
- [x] 6.2 headed 手动清单逐项过并记录观察：树显示活动 ws、展开/折叠、左键选中、右键复制路径（剪贴板回读）、右键在 Finder 显示、⟳ 拾取新文件、改 `.workspaces.json` 规则后 ⟳ 生效、无活跃 agent 空态、删根目录降级、`×` 关闭、左右槽拖拽
- [x] 6.3 文档收尾：`Docs/pending-decisions.md` 记入四项延后（配置界面立项时的 `agent-shell` 键盘条款修订、焦点重扫/FS 监听、惰性扫描、Windows reveal）；实现结论按需回写既有设计文档

## Workflow follow-up

- 实现完成并通过验收后，运行 `/opsx-archive` 归档本 change（含 delta specs 同步：`file-tree` 新主 spec 创建、`dock-layout`/`workspace-management` 修订合并）
