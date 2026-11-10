# Tasks

## 1. 打开系统骨架 (`src/property_detail_manager/`)

- [x] 1.1 建 `src/property_detail_manager/mod.rs`：中立 DTO `OpenObject{path, kind}` +
  `OpenHandler` trait（`claims` / `create_panel`，dyn-safe）+ 静态
  `HANDLERS` 注册表（v1 空表占位）+ `TerminalApp::open_object` 接缝
  （dedup `HashMap<String, PanelId>` 查询、无命中 no-op）；验证
  `cargo check` 通过，且无 handler 命中时 `open_object` 不创建任何签
  （无头测试 `open_object_without_handler_is_noop`）
- [x] 1.2 dedup 命中分支：已有 key → `dock_area.select_panel` 聚焦既有签，
  不新建；验证无头测试 `open_object_twice_yields_one_tab`（两次提交同
  path，签数 = 1 且 active 切到该签）
- [x] 1.3 `on_removed` 摘除钩子接缝：`open_tab_removed(panel_id)` 从 dedup
  摘除；验证无头测试 `closed_tab_can_be_reopened`（开→关→再开，签数
  第二次仍 = 1 而非跳过）

## 2. console 第一签不变量 + 标题

- [x] 2.1 `TerminalPanel` 实现 `title()` 返回固定 `"console"`（注释标明
  未来可随 session 名扩展）；验证无头测试
  `console_is_first_and_unclosable`：`open_object` 开 viewer 后中槽签序
  = [console, viewer]，且 `TerminalPanel::closable() == false`、viewer
  关闭后签序回落 [console]
- [x] 2.2 中槽顶栏常驻（2026-11-10 headed 验收追加需求, 覆盖原“单签不画”;
  方案 B 定案）: `TerminalPanel::title_bar=true` + Auto 样式 — 单 console
  签显示 30px 标题栏 (标题 console), 多签为 tab 条, 左右槽不变；验证无头
  测试 `top_bar_always_visible`（单签 render 后 label console 在场且
  center 下无 tabs-inner）+ 现有 pane/sidebar 回归全绿（`cargo test`）

## 3. EditorPanel + 首个 handler（只读 .rs）

- [x] 3.1 Cargo.toml 给 `gpui-kit` 启用 `syntect` feature；验证
  `cargo check` 通过且编译产物含 syntect（`cargo tree -i syntect` 可见）
- [x] 3.2 建 `src/property_detail_manager/editor_panel.rs`：`EditorPanel` 实体
  （`Entity<EditorState>` + 文件名 title + `WeakEntity<TerminalApp>`），
  实现 `Panel`（`panel_name "viewer"` / `closable true` /
  `title_bar false`）+ `Focusable` + `on_removed` 摘 dedup；打开时读文件
  → `replace` 灌入 → `language("rust")`；验证无头测试
  `viewer_shows_content_readonly`（打开含 Rust 源的临时文件 → 面板文本
  含原内容，`is_readonly() == true`，键入后内容不变）
- [x] 3.3 注册 rust handler（`claims`: kind==File && ext=="rs"）进
  `HANDLERS`；验证无头测试 `rs_file_opens_viewer_non_rs_noop`（`.rs` 开签
  且 tab 标题 = 文件名；`.md` / 目录提交 → 无新签）

## 4. 右槽钩子接入

- [x] 4.1 `WorkspacePropertySource::open` 实现：`FileTreeSource` 查权威树
  得 kind → 构造 `OpenObject` → 调 `open_object`（保留 `WeakEntity` 注入
  模式）；验证现有 `file_click_invokes_open_once` 改造为断言真实分发
  （stub 计数 → `open_object` 被调一次），`cargo test filetree_` 绿
- [x] 4.2 树点击后的焦点交回从"终端面板"改为"active tab 面板"
  （design D6-3：console active 时行为不变）；验证现有
  `tree_click_keeps_keyboard_with_session` 保持原断言通过（console
  active 场景 = 现状），并新增 viewer active 场景断言（焦点在
  `EditorPanel` 而非 TerminalPanel）

## 5. 焦点/键盘三落点（spec delta 对应）

- [x] 5.1 sidebar `SelectAgent` / `NewAgent` / `CloseAgent` 分支补
  `select_panel(console)`（design D6-2）；验证无头测试
  `sidebar_select_returns_to_console_tab`：viewer active 时点 sidebar
  agent → active 签 = console 且焦点在 TerminalPanel（一步聚焦）
- [x] 5.2 验证 `TerminalPanel::handle_key_down` 的 `focus.is_focused`
  守卫在 viewer active 时不转发（spec "Keystrokes follow the active
  tab"）；无头测试 `keystrokes_follow_active_tab`：viewer active 时注入
  按键 → session 收键计数不变、viewer 面板焦点保持
- [x] 5.3 IME 路由确认：viewer active 时 commit 不进 session（同 5.2
  链路，补 IME 分支断言）；验证 `cargo test` 全绿

## 6. 集成回归

- [x] 6.1 全量 `cargo test` 回归（现有 14+1 及 filetree/pane/sidebar
  套件保持绿）+ `cargo check` 无警告
- [x] 6.2 headed 手动验收清单执行并记录（2026-11-10 三轮真机验收通过, 用户确认）：点 `.rs` 开签高亮/只读、重复点
  聚焦不重开、关签可重开、sidebar 切回 console、点 `.md` 无反应、
  console 恒第一签不可关、viewer 上打字不进 pi
- [x] 6.3 `openspec validate center-slot-multi-tab` 通过（无新 INFO）；
  确认前置 `right-dock-file-tree` 归档状态并在归档顺序上标注

## Workflow follow-up

- 先归档 `right-dock-file-tree`（file-tree 主 spec 就位），再归档
  `center-slot-multi-tab`（其 file-tree delta 才能挂载）
- 归档后跑 `openspec list --specs` 确认 `center-tabs` 能力已入主清单
