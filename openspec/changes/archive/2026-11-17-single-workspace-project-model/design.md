# Design

## Context

See proposal.md for motivation. Constraints discovered during exploration that
shape this design:

- 现有模型：`TerminalApp` 持有 `workspaces: Vec<Workspace>`，`Workspace` 只有
  `path / collapsed / agents / next_agent_idx`（`src/workspace/mod.rs`）。
  会话实体是 `Entity<AgentSession>`，`AgentEntity::ws_path()` 暴露 cwd。
- 持久化：`src/workspace/store.rs` 读写 app 根的 `.workspaces.json`
  （`workspaces` 列表 + 按 workspace 路径键控的 `file_tree` 规则区段），
  且有"写回保全区段"的既有纪律。
- 左槽渲染是每帧快照模式（`sidebar::snapshot()` → `WorkspaceRow/AgentRow`
  值快照），事件统一走 `SidebarEvent` 漏斗到 `TerminalApp::on_sidebar_event`
  单一入口（design D3/D7，`Docs/pending-decisions.md` 有记录）。
- 右槽 file-tree 的扫描/过滤/缓存契约全部保留；本 change 只改规则的读取来源。
- spec 纪律：行为变更先落 delta（本次 6 份，见 specs/），归档时同步主 spec；
  归档过的 change 不回改。
- 启动时序现实：`main.rs` → 窗口创建在前，错误对话框 + 退出是既有契约
  （workspace-management "Corrupt file blocks startup"），沿用同一路径。

## Goals / Non-Goals

**Goals:**

- 单工作区生命周期（恢复 / 选择 / 创建 / 切换门控）与
  Solution → Project → Module 领域模型在**一个 change** 内落地，
  左槽切到 Project/Agent 树。
- 领域模型是**纯数据层**：不依赖 gpui，可无头单测（与现有测试套件同构）。
- 加载边界统一：启动恢复、手动切区、显式重载共用同一条
  "校验 → 失败则错误框+退出" 的读路径。
- 旧 `.workspaces.json` 平滑退役（历史 + 规则一次性迁移，不破坏用户数据）。

**Non-Goals:**

- 右槽树不改 root、不加 Project/Module 层（M3 独立 change）。
- Solution 管理界面只做"空工作区添加"一条路径，不做编辑/删除/重排。
- 错误修复/解析辅助（对话框只报错，不提供 fix-it）。
- 会话持久化、worktree、服务层注入、主管 Agent —— 全部后置。
- 跨工作区并行、多窗口 —— 单实例假设不变。

## Decisions

### D1. 领域模型归 Solution 实现所有：注册表 + 一次性展开

**Decision**: `balabala.solutions.json` 的 item 只有 `{ "type": ..., ... }`
两层结构；`SolutionType → 实现` 走**静态注册表**（v1 一个 `simple`）。
加载时把 `Vec<SolutionItem>` 一次性展开成
`Vec<Project>`（每个 Project 内含 root Module）。"一个 path 怎么铺出
Project/Module 结构"完全由 Solution 实现内部决定。

**Why**: 讨论过"文件系统推导 vs 配置声明"两条路线，都把组织逻辑放进了
全局 schema——推导把 LSP 分区绑死在目录约定上，声明让 solutions 文件
膨胀成第二份配置。归 Solution 之后**配置只有一份**，将来
`cargo-workspace-solution`（扫 Cargo.toml）或声明式 solution 只是注册表
加一项，schema 和加载器不动。

**Alternatives rejected**: 全局 module 声明段（配置双源、树与磁盘漂移）；
直接扫目录推导（组织策略不可选，LSP 边界表达不了）。

### D2. 纯数据层模块 `src/project_model/`，应用层只持 `Option<WorkspaceModel>`

**Decision**: 新建不依赖 gpui 的模块：

```
src/project_model/
  mod.rs        WorkspaceModel { marker, solutions, projects }
  solution.rs   SolutionItem / SolutionType 注册表 / SimpleSolution
  project.rs    Project { name, root, modules } / Module { name, parent, children }
  validate.rs   load() -> Result<WorkspaceModel, ModelError>
                (解析、未知 type、死 path、DAG 环检测)
```

`TerminalApp` 持 `model: Option<WorkspaceModel>`；左槽 render 从 model
快照行（沿用现有 snapshot 模式）。Project 的稳定身份 = root 路径
（`PathBuf`），与 sidebar 事件载荷"只带稳定身份"的既有纪律一致。

**Why**: 校验/展开是纯函数，无头测试直接覆盖 spec 场景（项目里
`cargo test` 已经是无头套件）；gpui 只消费快照，不进加载路径。
DAG 环检测对 v1 是空转，但作为 `validate.rs` 的独立函数先立骨架
（proposal 已声明"结构先立、校验随后续 solution 激活"）。

**Alternative rejected**: 把 model 塞进 `workspace::Workspace`（两个概念
耦合进同一结构，右树/未来服务层也要读 model，会反向污染 workspace 生命周期）。

### D3. 文件契约与创建原子性：marker 最后写

**Decision**: 创建顺序 = 先写 `balabala.solutions.json`（`[]`），成功后
再写 `.balabala.workspace.json`（`{"version": 1}`）。marker 是"工作区已
成立"的唯一真值：marker 在 = 两文件齐；marker 不在 = 目录还不算工作区。

**Why**: 任一步失败都不会留下"半个工作区"——失败后目录里最多残留一个
孤立 solutions 文件，重试创建直接覆盖；反过来先写 marker 会出现
"solutions 缺失的合法工作区"，与 spec 的创建语义（两文件一起出现）冲突。
读侧因此只需看 marker 一个文件，不用交叉对账。

**Detail**: `file_tree` 规则区段进 marker 文件（`.balabala.workspace.json`
= 身份 + 配置，`{"version": 1, "file_tree": [...]}`，规则**不再按路径键控**
——单工作区天然免键）。空区段按现有惯例 `skip_serializing_if` 省略，
保持字节稳定。这也回答了"规则搬去哪"：不引入第三个配置文件。

### D4. 历史存储：全局应用目录，带 `legacy_seeded` 标志

**Decision**: 用 `directories` crate 解析平台应用数据目录，存
`workspace_history.json`：

```json
{ "version": 1, "legacy_seeded": true, "recent": ["/a/ws", "/b/ws"] }
```

`recent` 按最近打开顺序去重，封顶（建议 10 条）。启动时一次性迁移：
若 `legacy_seeded` 为 false 且 app 根存在 `.workspaces.json` → 读入，
合并 `workspaces` 进 `recent`，置 `legacy_seeded = true`，此后永不再读
（legacy 文件永不被改写）。`file_tree` 规则迁移不同步做：打开某工作区时
若其配置缺 `file_tree` 区段，再查 legacy 文件按该路径补拷一次
（补完写入工作区配置，legacy 只读）。

**Why**: 单工作区后"列表"只能是 MRU；放全局目录才与"配置都在工作区里"
不冲突（历史本身不属于任何工作区）。`legacy_seeded` 分开记，是因为规则
补拷按需发生、可能晚于历史播种——两个迁移节奏不同，一个标志管两件事
会漏。历史文件损坏的处理与既有纪律一致：当作无历史（回空态），不退出
——历史是便利数据，不是身份数据。

**Alternatives rejected**: 历史存 marker 文件（把应用级状态写进工作区，
违反 D3 的文件职责划分）；每次启动扫 legacy（迁移永远不结束，用户手改
legacy 会意外改变行为）。

### D5. 启动与加载时序：窗口先行，model 异步就位

**Decision**:

```
main()
  └─ open window (空态骨架先渲染)
       ├─ history 有档 ──> epoch++ , spawn load(path)
       │     ├─ marker 缺/目录没 → 回空态 (不报错)
       │     ├─ marker 坏 / model 校验失败 → 错误框 → exit
       │     └─ Ok(model) → epoch 匹配才 commit → notify 重渲
       └─ history 无档 → 停在空态选择器
```

**Why**: spec 要求"主窗先行"（路线 B），且错误框+退出契约要在恢复路径
上生效。**epoch（加载代号）**是防竞态的关键：用户在 load 未完成时点了
切换/取消，旧 load 的结果直接丢弃（`epoch` 不匹配不 commit），避免
"A 的 model 装进 B 的窗口"。gpui 事件监听器内不做阻塞 IO（现有 design
D3 纪律），所以解析/读盘在 spawn 的任务里，完成后 `weak.update` 提交。

### D6. 切换门控是显式状态机，挂在 `on_sidebar_event` 漏斗上

**Decision**: `SidebarEvent` 扩三个载荷，仍走单一入口 match：

```rust
enum SidebarEvent {
    // ... 现有 6 种
    SwitchWorkspace,            // 顶栏切换入口
    ToggleProjectCollapse(PathBuf),
    AddSolution,                // 空工作区添加入口
}
```

切换门控放在漏斗入口处统一判断：

```
SwitchWorkspace
  ├─ 任意 session ∈ {Starting, Running} ──> Notice("N active, close first") 停
  ├─ 仅 Exited ──> Confirm("exited 屏幕将丢弃") ── 取消: 停
  │                                     └─ 确认: drop 所有 session 实体 → picker
  └─ 无 session ──> picker
```

**Why**: 与"事件漏斗单一分发"的既有设计一致，门控逻辑一处可测；
不引入命令队列/事件总线（pending-decisions 已明确暂不做）。确认对话框
复用启动错误框同级的模态路径，picker 复用 `rfd::AsyncFileDialog` +
`cx.spawn`（D8 已有先例，不阻塞主线程）。

### D7. 左槽树结构：三段式骨架，行数据全量快照

**Decision**: 左槽自上而下固定三段：

```
[ switch entry (工作区短名 ▾) ]        <- 永远在顶
[ project row (▾ foo)   x ]            <- model.projects 快照
      foo/pi-1  ●running   x
      foo/pi-2  ●exited    x
[ [+ agent] ]                          <- 每个 project 行底部
[ + add solution ]                     <- 仅 projects 为空时渲染
[ 空态选择器 / 空 solutions 提示 ]        <- 仅无工作区或空工作区
```

每帧 `snapshot(model, sessions)` 产出拥有值的行数据（现有模式不变）；
project 行折叠态存 `HashSet<PathBuf>` 于 app（易失，不持久化——对齐
"collapse state 不持久化"的既有立场）。

**Why**: 三段顺序把"切换 > 结构 > 会话 > 动作"固定下来，render 无需
分支判断嵌套状态；折叠态易失是刻意的——工作区目录换机器/重新克隆后
折叠态本来就无意义，持久化它是 pending-decisions 里明确不想背的债。

### D8. cwd 与命名的接线点

**Decision**: `AgentEntity::ws_path()` 更名为语义正确的 cwd 暴露
（实现层改读 `project.root` 而非 workspace 路径）；`Workspace::new_agent_name`
的 per-workspace 计数器移到 Project 结构上（per-project
`next_agent_idx`），显示名 `<project>/pi-N` 不变。

**Why**: sidebar 展示契约（`foo/pi-1`）恰好与路径末段同构，改锚点后
显示零变化；计数器跟人走（project），不跟容器走（workspace），
与"agent 挂 Project"的模型一致。

### D9. 右槽过渡期行为（明确不改）

**Decision**: file-tree 的扫描根仍是"活动 workspace 目录"，行为除规则
读取来源（store → workspace 配置文件）外逐字不动；右树与左槽在本 change
结束后存在短暂的视图模型不一致（左 Project 视角 / 右目录视角），M3
change 统一为 Project/Module/File。

**Why**: 右树现有缓存/失效/O(1) 通知契约（file-tree spec 5+ 条）全部
有效，动 root 是纯破坏面；把不一致显式写进 proposal 而不是偷偷改，
M3 对齐时 spec delta 才有清晰的对照基线。

## Risks / Trade-offs

- [workspace-management 是近全量重写，7 条 REMOVED 的行为里"增删/去重/
  折叠"可能有隐性依赖] → delta 每条 REMOVED 都给了 Reason+Migration；
  实现时按 tasks 逐条核对旧测试，删测试也要留注释指向新归属。
- [死 path / 坏 solutions 一律"错误框+退出"对可移动盘用户偏硬] →
  已被确认接受（决策 4），错误修复/解析列 Open Question 后置；
  对话框文案必须点名文件与原因，让手工修复可行。
- [加载异步化引入竞态（D5 epoch）] → commit 只发生在 epoch 匹配时；
  tasks 里配一条无头测试：发起 load 后立即切区，断言旧结果被丢弃。
- [一个 change 同时动生命周期+领域模型，体积偏大] → 已与用户确认合并
  （拆开会产生尴尬中间态）；tasks 按 D1-D3（模型层）/ D5-D6（生命周期）/
  D7-D8（UI 接线）分组，可分批 review。
- [legacy 迁移规则补拷需要"读 legacy + 写工作区配置"跨目录写] →
  legacy 永远只读；补拷失败按 D 写失败契约走红条，不阻塞打开。
- [per-project 计数器随 model 重建归零（不持久化会话）] → v1 会话本就不
  持久化，计数器只在工作区打开期间有意义；重启后 `foo/pi-1` 重现是
  spec 允许的（会话未持久化条款不变）。

## Migration Plan

1. **读侧先行**：store 读路径支持新旧两处（legacy 种子 + workspace 配置），
   旧 `.workspaces.json` 只读不写；发布即迁移完成，无第二阶段。
2. **回滚**：change 归档前整体可回（git revert）；归档后 legacy 文件仍在
   app 根，恢复旧版本即可继续用，无数据损失面。
3. **测试过渡**：现有 `.workspaces.json` 相关测试改为 legacy 种子测试与
   workspace 配置读写测试两组；pane/tree 的无头测试不受影响（右树行为未变）。

## Open Questions

- **错误修复/解析辅助**：对话框是否内嵌"打开文件/重载"动作 —— 不改
  spec（spec 只约束"报错并退出"的时机），可在实现阶段以纯 UI 增量补。
- **Solution 编辑/删除**：移除一个 solution item（连带 Project 行消失、
  其下 exited 会话丢弃）的交互与门控 —— 会改 project-model spec，
  等管理界面立项时一并提 delta。
- **历史封顶条数与是否显示二级历史列表**（切换入口下拉 vs 只有最近一条）
  —— 影响的是 UI 增量，切换入口的行为契约（picker 路径）已定。
- **`.balabala` 前缀的缓存/子目录命名** —— 本 change 不创建，随
  worktree/cache change 定名。
