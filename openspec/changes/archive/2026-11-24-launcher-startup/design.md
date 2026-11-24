# Design

## Context

启动链路今天是一条直线：`main.rs` 开唯一窗口 → `TerminalApp::new_inner` 读历史 →
有历史就 `begin_open(MRU)` 自动恢复，没历史就留 sidebar 空态当选择器。选择器和
切换入口共用一个控件，切换入口背后挂着三支门控状态机（`switch_gate` /
`GateDecision` / `count_sessions`，17 处引用）。

约束（详见 proposal.md）：启动要变成显式关卡——只出 Launcher，不可跳过，关窗即中断，
且 Launcher 存续期间不启动任何其他逻辑。A1 已定：Launcher 只选路，load 留在主窗，
失败仍是错误框 + 退出。(c) 已定：Launcher 自己先校验，校验得过的才交棒。

两个既有事实塑造了方案：

- `QuitMode::LastWindowClosed` 是 2026-11-10 实测 `Explicit` 残留僵尸进程后设的。
  Launcher 阶段它是唯一窗口，**关窗即退的语义天然成立**，不用改。
- `ServiceLayer::bootstrap` 不需要 window，但它扫描 plugins/ 并装载三个服务——
  这正是"Launcher 期间零其他逻辑"要挡掉的东西。

## Goals / Non-Goals

**Goals:**

- 启动序列拆成两段：Launcher 段（只读 fs）→ 主窗段（bootstrap + dock + load）
- 校验前移到 Launcher，让 `surface_fatal` 的 exit 只覆盖"交棒后才失效"的竞态
- 切换口与三支门控整体退场，`new_inner` 不再读历史
- 菜单栏在 Launcher 阶段即存在，主窗装配后按既有 rebuild 条款补齐

**Non-Goals:**

- 不做会话内切换工作区（已明确删除，不是延后）
- 不改 `begin_open` 的 epoch 防竞态、`commit_open` 的历史 push、`history_path` 字段
- 不动 solutions 文件的加载语义——它仍归主窗，失败仍退出（见 D4）
- 不做历史条目的后台预检置灰（(a) 叠加项，本轮只要 (c) 的底座）
- 不引入第二窗口生命周期管理以外的多窗能力

## Decisions

### D1: Launcher 是独立窗口，不是主窗内的一个状态

`main.rs` 先 `open_window` Launcher；选定 path 后再 `open_window` 主窗
（`TerminalApp::new` 在其中构造），最后关 Launcher。

- **为什么不是窗内状态**：`TerminalApp::new_inner` 一次性做完 bootstrap + dock +
  theme + menu，拆不开；窗内状态意味着 Launcher 阶段这些要么已经跑了（违反"零其他
  逻辑"），要么主窗构造要延后到选定时（等于还是要第二次装配）。独立窗口让两段
  各自完整。
- **顺序必须是先开主窗、再关 Launcher**：`QuitMode::LastWindowClosed` 下，中间
  出现零窗口会直接退进程。`Window::remove_window()` 只置 `removed = true` 延后
  处理，与 `open_window` 同批执行是安全的。
- **代价**：生产代码首次出现第二窗口，无先例（`property_detail_manager` 的
  `open_window` 是 `#[cfg(test)]`）。Launcher 是独立的轻量实体，不持有
  `TerminalApp`，两窗不共享状态——只通过选定的 path 交接。

### D2: `new_inner` 收 path 参数，历史读取整体搬 Launcher

| 搬去 Launcher | 留在 `new_inner` |
|---|---|
| `History::load_at` + `seed_legacy` | `history_path` 字段（`commit_open` 要 push） |
| `if let Some(path) = restore` 自动恢复 | `cx.bind_keys` |
| picker → `marker_status` → 确认创建那 ~70 行 rfd 胶水 | `shell::build` / theme / `menu::install` |

- **为什么 `history_path` 留着**：它是 `commit_open` 写历史的通道，读和写分属两段。
- **legacy 播种跟着搬**：播种是"为 Launcher 准备历史"，属于读侧；且它必须在 Launcher
  呈现历史之前完成（spec 已钉）。
- **`OpenWorkspace` / `SwitchWorkspace` 事件与 `request_workspace_open` 的 gated
  分支删除**；`SidebarEvent` 从 7 变体降到 5。

### D3: `OpenProbe::Empty` 与 `OpenOutcome::Empty` 并入 Failure

今天三臂（`Empty` / `Failure` / `Valid`）中 `Empty` 是"回空态不报错"，服务对象是
自动恢复的目标失效。自动恢复退场后，`Empty` 只剩一个来源：**Launcher 校验过之后
到主窗 probe 之间的毫秒级竞态**。既然 Launcher 已经校验过，再失效就该报错退出。

- **结果**：两个枚举各塌成两臂，`commit_open` 的 `Empty` 分支删除，
  `workspace-empty-hint`、`snap.ws.is_none()` 分支成为不可达代码，一并删除。
- **为什么不是保留空态**：切换口已删，空态无任何可点控件——提示用户"choose a
  directory"却没有入口，是死胡同。
- **代价**：加载期那几十毫秒会渲染"无 workspace"提示的问题今天就存在（不是本
  change 引入），但删除后该窗口期渲染的是 sidebar 标题加空列表，文案错误的问题
  随之消失。

### D4: 校验边界前移到 Launcher，`surface_fatal` 只留给竞态

这是 (c) 的落地，也是本 design 最关键的分界：

```
Launcher 段 (只读 fs, 无服务)
  marker_status(选定路径)
    ├─ 目录不存在 / marker 缺失 / marker 解析失败
    │     → Launcher 内就地报错, 留在 Launcher, 进程不退
    │        (marker 缺失 → 走确认创建)
    └─ 合法 → 开主窗 → 关 Launcher
主窗段
  TerminalApp::new_inner (bootstrap + dock + menu)
  begin_open(path) → probe → solution load
    ├─ 竞态失效 / solutions 解析失败 / 服务不可达
    │     → surface_fatal: 错误框 + 退出   ← 唯一的 fatal 启动路径
    └─ 成功 → commit_open(Ready)
```

- **为什么这样分**：`surface_fatal` 的 exit 语义前提是"主窗已开、无处可退"。
  Launcher 还没交棒时失败，关卡自己把用户踢出去等于关卡失效——这正是"不允许跳过"
  要防的。
- **两层校验不重复，是分工**：Launcher 做**建议性预检**（给 UX：报错留在原地、
  创建前确认）；主窗做**权威探测**（给契约：`probe_open` + solution load）。
- **solutions 文件仍归主窗**：解析它需要 solution 服务，而 Launcher 期间服务未装载
  （D1/Non-Goal）。所以"marker 坏"和"solutions 坏"的后果不同——前者留在 Launcher，
  君者退出。spec 已分别写明。

### D5: 菜单栏分两步装

`menu::install(cx: &mut Context<TerminalApp>)` 依赖 `TerminalApp`，Launcher 阶段
没有它。`set_menus` 本身是 `App` 方法。

- **做法**：Launcher 期直接 `cx.set_menus(build 中 debug 块之前的 Edit 部分)`；
  主窗 `new_inner` 尾部照旧整根重推带 `Debug` 的完整版。
- **为什么不是不装**：`app-menu-bar` spec 要求 "at startup in all build profiles …
  at least one menu at all times"。
- **为什么不用 `menu::install`**：它的 debug action 监听要
  `cx.weak_entity::<TerminalApp>()`。拆开 `set_menus` 与监听注册会污染既有 D11
  装配文档，不如按构建期分两次调用。

### D6: 测试迁移

- 4 个 startup 测试（`startup_restores_most_recent_workspace` /
  `startup_corrupt_marker_records_open_failure` /
  `startup_missing_target_falls_back_to_empty_state` /
  `startup_restore_load_failure_shows_error_and_no_workspace`）的被测行为搬到
  Launcher 测试：历史呈现、选定校验、关窗即退。
- `empty_state_renders_picker_affordance` 删除（空态不可达）。
- `full_chain_create_add_agent_and_gate` 的链路起点断言与门控尾部删除。
- `app.rs` 两处 `request_workspace_open(true, …)` 门控断言删除。
- `open_app_with_history` 的 23 个调用点：多数用 `fresh_history_path()`（空历史、
  原本不触发恢复），`new_inner` 去掉 restore 分支后改为显式传 path 或 `None`，
  语义更直白。

## Risks / Trade-offs

- **[两窗交接的顺序错误导致启动即退]** → 先 `open_window` 主窗、同批
  `remove_window()` Launcher；把顺序写进 tasks 的验收项，配无头测试断言
  Launcher 关闭后主窗仍存在。
- **[Launcher 阶段无菜单或菜单残缺]** → D5 明确装 `Edit`；补一条 render 级断言
  （菜单栏在 Launcher 期至少一个菜单）。
- **[校验前移造成 marker 读两次]** → 一次在 Launcher（建议性），一次在主窗
  `probe_open`（权威）。重复的是 fs 读，不是逻辑；接受，换取两层职责清晰。
- **[生产首例双窗口]** → Launcher 不持有 `TerminalApp`、不共享状态，只交接 path；
  风险局限在关窗顺序，已在上条缓解。
- **[死条目曾可静默无反应，现在会就地报错]** → 这是行为收紧，UX 上是改善；
  但既有测试若断言"点了没反应"需同步改写。

## Migration Plan

单个 change 内完成，无分阶段部署。回滚 = 还原 `main.rs` 启动序列与
`new_inner` 的历史读取。spec 层面 `launcher` 为新增能力，其余为修改/删除，
`openspec archive` 时一并合入主 spec。

## Open Questions

- **历史条目是否后台预检置灰**（(a) 叠加项）：本轮只做 (c) 的"点了才校验"。
  置灰需要渲染前 probe ≤10 个 marker，成本低但引入异步渲染态，留待有反馈后再定。
  不影响本 change 的 spec 与任务拆分。
