# Tasks

## 1. 门禁 Spike（临时分支，验证后丢弃）

- [x] 1.1 开临时分支搭最小 DockArea 集成（mimic 面板复刻四个挂载点：
      `key_context("Terminal")` + `track_focus` + `on_key_down` + canvas
      `handle_input` + `on_click` focus），跑无头 T1-T5/I1——验证焦点逐步
      后 `window.focused() == pane handle`、`dispatch "tab"` 触发
      TerminalTab handler、注入 commit 文本到达 input handler
- [x] 1.2 headed 手动一次（约 5 分钟）：I2 中文 IME 候选框出现/位置合理/
      commit 上屏/preedit 不显示；T6 真实 Tab 键在中文输入前后仍转发
- [x] 1.3 同分支顺带 V1-V6：DockArea 测试窗口跑通、`title_bar() -> false`
      后 `+ new workspace` 置顶（含 `inner_padding()` 确认）、失效链
      （poll→notify→两面板刷新）、resize 拖拽/拖出跟踪/clamp、
      `toggle_dock` 关开全程、`find("terminal-grid")` 可达；
      结论回写 `Docs/module-split-design.md` §10
- [x] 1.4 判定门禁：全过 → 丢弃分支进入步骤 2；任一失败 → 停止迁移，
      按 §10.4 预案（F1 焦点兜底 → F4 回退自研 Dock）探索出方案后重启

## 2. ID 化（与 DockArea 无关，行为保持）

- [x] 2.1 `active: Option<(usize, usize)>` → `Option<EntityId>`，同步改
      `create_agent`/`close_agent`/`send_key`/`send_text`/`close_workspace`
      的补偿逻辑与 `adaptive_grid_dispatch` 的注入语句（先取 id 再 move）；
      验证：`cargo test` 全绿 + 手动增删 workspace/session 切换冒烟
- [x] 2.2 事件载荷去索引：sidebar 行数据与 listener 捕获改用
      `PathBuf`/`EntityId`（`AgentRow` 的 idx 字段替换）；验证：
      `cargo test` + 连续快速点击不同行无错位（闭包捕获不过期）

## 3. SidebarPanel 实体化（先挂现有布局，不碰 DockArea）

- [x] 3.1 `ui/sidebar.rs` → `sidebar/mod.rs`：`SidebarPanel` entity 实现
      `Panel`（`panel_name="sidebar"`、`title_bar() -> false`、
      `closable() -> false`、`zoomable() -> false`、`Focusable`、
      `EventEmitter<PanelEvent>`），持有 `WeakEntity<TerminalApp>`；
      listener 改经 `SidebarEvent` 漏斗进 `TerminalApp::on_sidebar_event`；
      以普通 child 挂回现有布局左槽；验证：`cargo test` 全绿 + 手动
      sidebar 全部按钮（新建/折叠/关闭/`[+ agent]`/选中）行为不变，
      且 `+ new workspace` 仍是 sidebar 顶部第一个可交互元素（spec）
- [x] 3.2 `TextTooltip`/`WorkspaceRow`/`AgentRow` 随迁，一步切焦点逻辑
      收进 `SelectAgent` handler（`active = id` + 焦点交 terminal 面板）；
      验证：悬停 tooltip 显示绝对路径；点 agent 行即能打字（spec
      one-step focus）

- [x] 3.3 notify 转发机制（spike §10.5 实测前提：面板自读不重建）：
      drain 循环（session 产出 → notify 展示面板）+ model 漏斗（create/close/
      toggle/persist → notify 相关面板）接线；本步先转 sidebar，步骤 4 起把
      terminal 面板加入转发列表；验证：折叠/新建/关闭即刻反映，agent 输出时
      sidebar badge 同步刷新（`cargo test` + 手动）

## 4. TerminalPanel 实体化（先挂现有布局，不碰 DockArea）

- [x] 4.1 `ui/pane.rs` → `pane/mod.rs`：`TerminalPanel` entity 实现
      `Panel`（`panel_name="terminal"`、`title_bar/closable/zoomable ->
      false`），原 `pane_focus` 迁入为 `Focusable::focus_handle`；
      `#terminal-pane` 整块 render 移入（`key_context("Terminal")` +
      `track_focus` + `on_key_down` + `on_action(TerminalTab)` + `on_click`
      + IME canvas）；以普通 child 挂回现有布局中心槽；验证：
      `cargo test` 中 `adaptive_grid_dispatch` 绿（依赖
      `window.find("terminal-grid")`）
- [x] 4.2 自由函数与单测随迁（`render_empty`/`status_bar`/
      `terminal_grid`/`grid_metrics`/`build_grid`/`PaneData`/
      `MIN_GRID_COLS`/`MIN_GRID_ROWS`），导入路径改
      `crate::pane::...`；验证：网格单测与 `app.rs` 测试内
      `grid_metrics` 导入全部编译并 `cargo test` 全绿
- [x] 4.3 输入路径迁移：面板 listener 内焦点检查改 `self.focus
      .is_focused(window)`，`send_key`/`send_text`/`propagate_grid_size`
      留 `TerminalApp` 经 weak 调用，`EntityInputHandler` impl 原样留
      `TerminalApp`（canvas 内 `weak.upgrade()` 注册）；验证：手动点
      pane 得焦、可打印字符转发、Ctrl 组合不转发（spec）、中文 IME
      commit 上屏

## 5. 装配切换（接入 DockArea）

- [x] 5.1 新建 `shell/mod.rs`（`shell::build`：`DockSkin::dock_area`
      + `add_panel_view(panel_handle(sidebar/right), Left/Right, Some(px(240.)))`
      + `set_center(DockLayout::tabs().panel_view(panel_handle(terminal), cx))`
      + `set_dock_collapsible(Left, false)`——**必须 panel_handle 包装**（裸
      entity 绕过 `title_bar()=false`，每槽多画 30px 标题栏，spike §10.5；
      顺序：先 add 建 dock 再 set_collapsible）与
      `shell/placeholder.rs`（`RightPlaceholderPanel`：header 标题 + `×` +
      body 提示）；`TerminalApp::new(window, cx)` 签名变更，`main.rs` 与
      测试直传 `_window`；验证：`cargo build` 成功，窗口出现左 dock
      （sidebar）/中心（终端）/右 dock（占位）三槽布局
- [x] 5.2 `TerminalApp::render` 换为外壳容器 + `dock_area` 子节点；
      占位 header `×` 调 `toggle_dock(Right)`（本轮不注册重开键位），
      **关闭后显式把焦点交回 terminal pane**（spike E2：点 × 会把焦点交给
      即将隐藏的面板）；
      验证：逐条走 `specs/dock-layout/spec.md` 场景——启动右 dock 可见、
      左/右拖拽调宽且网格重算、`×` 关闭零渲染中心变宽且会话内无重开
      入口、重启恢复默认宽度、左 dock 无任何关闭入口
- [x] 5.3 `adaptive_grid_dispatch` 适配装配后布局（`TerminalApp::new`
      签名、`active` 注入、导入路径；断言为相对值，必要时只调测试窗口
      尺寸不改断言）；验证：`cargo test` 全绿，含 resize 递增与
      去重断言
- [x] 5.4 `ui/widgets.rs` 拆分：`TextTooltip` 等共享小件归位，
      `ui/mod.rs` 保留颜色映射；验证：`cargo test` 全绿 + 悬停
      tooltip 正常

## 6. 集成验收

- [x] 6.1 spec 回归清单逐项过：`cargo test` 全绿；一步切焦点（spec
      one-step focus）；Ctrl 不转发；中文 IME（commit 上屏、preedit
      不显示）；Tab 键转发（D5 路由）；`+ new workspace` 顶部置顶；
      失效链双通道（agent 输出滚动 + sidebar badge 实时）；右 dock
      全部场景（dock-layout spec）；验证：清单逐项打勾
- [x] 6.2 实现差异与 spike 结论回写 `Docs/module-split-design.md`
      （§10 结论、开放问题 Q1/Q4 已定案与 Q5 现状）；验证：文档更新完成
      且与实现一致

## Workflow follow-up

- Archive the change after the project's review requirements are satisfied.
- Verify the archived result.
