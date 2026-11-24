# Design

## Context

左槽面板 `sidebar` 同时承载 workspace 入口、project 行、agent 行，但
solution 这一层在视图里不可见——`TreeSnapshot.solutions` 只有 SolutionConfig
表单的预检在读，`ConfigureReq` 契约与幂等工具齐全却**零 GUI 调用点**。

节点操作的描述方与归属方没有分离：asset 侧是 `AssetManager::menu()` 里一个
`match func_id`，sidebar 侧是 `SidebarEvent` 枚举本身（7 变体，中央 match 分发）。
`Docs/pending-decisions.md` 的"统一 sidebar 事件处理器"条目早已记下这个痛点，
当时的处方是 `Command` trait + `EventBus`——本 design 走的是另一条路：**让服务
注册操作，视图只转发**，因为操作的真值（solution 有哪些开关、当前值是多少）
本来就在服务手里。

既有约束：

- `TreeSnapshot` derives `Debug, Clone, Default, PartialEq, Eq`，
  `last_handoff` 靠 `==` 做 O(1) 失效检测。
- 菜单构建是**同步**的（`context_menu` 闭包内直接调），不能等 ask 回来；
  而 `tree_view()` 是同步读镜像——所以描述符必须随快照下传。
- `Payload` 是 `#[derive(Clone)]` only，无 `PartialEq`/`Default`。
- spec 层 `sidebar` 提及 11 个 requirement 块 + 1 个标题含 `Sidebar`。

## Goals / Non-Goals

**Goals:**

- solution 成为左槽一等节点，其配置开关获得 UI 入口
- 树契约收敛为单一 view model，`tree`/`handoff` 降为派生
- 操作的描述与归属回到服务，视图只渲染与转发
- 写入口收敛为一个 kind，回执路由摆脱 UI 文案

**Non-Goals:**

- worktree 节点与 `create worktree` 的实际落地（本轮只留层级与参数通道）
- asset-view 迁入本机制（见 Open Questions ③）
- 会话/折叠态持久化（维持易失立场）
- i18n——label 进契约是单语言阶段的取舍
- 改动 `handoff` 交树的资产注册语义

## Decisions

### D1: `hierarchy` 成为唯一 view model，`tree`/`handoff` 降为派生

```rust
pub struct TreeSnapshot {
    pub solutions: Arc<[SolutionItem]>,   // 文件 DTO 原样, create_is_noop 要吃它
    pub hierarchy: Arc<[SolutionNode]>,   // 唯一展开数据
}
impl TreeSnapshot {
    pub fn tree(&self) -> ProjectTree      // 派生: 展平
    pub fn handoff(&self) -> ProjectTree   // 派生: 过滤注册开关
}
```

- **为什么不是加第四个字段并存**：`solution-overview` 那个 change 的全部动机
  就是"两份本地投影 ⇒ 漂移窗口 + 读状态时该信哪个"。加并行字段是同一个坑。
- **为什么 `solutions` 留作字段而非也派生**：它是 `items` 的原样回附（文件
  DTO），且 `SolutionConfigPanel::prechecks` **每个 keystroke** 调
  `create_is_noop(&t.solutions, …)`——派生它意味着每次按键构造一个
  `Vec<SolutionItem>`。它是 hierarchy 的**输入**不是视图，留字段不破单一
  view model。
- **涟漪**：非测试读点仅 4 处（`app.rs:727` 预检、`app.rs:766` 交树、
  `sidebar:137`、`solution_config:65`），测试 30 处，构造点 2 处。
  成本几乎全在测试改写。
- **`last_handoff` 顺手收窄**为只比 `handoff()` 投影：名副其实，省掉全量
  clone，且提前拆掉一个雷——描述符一旦上节点，"标签变了但交树没变"会触发
  无谓的全量资产重枚举。

### D2: solution 行 = 分组头 + 配置挂载点

```
foo · simple        ← solution 行 (label 带类型标记, tooltip 完整 path)
│                    右键: 配置开关菜单; 左键: 纯折叠
└─ foo              ← project 行 (label = name, 今天不变)
    ├─ ● foo/pi-1
    └─ [+ agent]
```

- **为什么必须带类型标记**：`expand_item` 里 `root = item.path()`、
  `name = file_name(root)`——v1 `simple` 的 solution path 与 project root
  **是同一目录**，两行 label 会一模一样。类型标记是唯一视觉区分。
  （备选"直接显示完整 path"因侧栏窄被否。）
- **为什么左键纯折叠、无打开语义**：对齐 asset 树 `project://` 分组头的
  既有立场（无菜单/无打开语义）；打开动作属于 project 行下面的东西。
- **身份必须命名空间化**：裸路径会让 solution 行与 project 行撞 id，坏掉
  展开集、右键查找、观测锚点。asset 树已踩过一次并用 `project_id()` 解决
  （`assets/state.rs` 注释原话："若两行都用裸路径会撞 id"），照抄该模式：
  `solution://<path>` / 复用 `project://<path>` / 后续 worktree 与文件行用
  裸路径。
- **折叠态**：新增 `collapsed_solutions` 与 `collapsed_projects` 并列，
  同为易失不持久化（对齐既有立场）。

### D3: 描述符随快照下传，label 归服务

```rust
pub struct ActionDesc { id: String, label: String, checked: Option<bool> }
pub struct ActionInvoke { node: Option<String>, action_id: String, args: Option<Payload> }
```

- **为什么必须随快照**：`context_menu` 闭包是同步的，等不了 ask。而
  `tree_view()` 同步读镜像——描述符搭快照的车，菜单构建零往返。
- **为什么 label 归服务（契约携带 UI 文案）**：`(ii) 只带 id、view 查表`
  会让 view 必须认识每个 id 才能显示——**view 又耦合回服务**，正是本 change
  要拆的东西；且插件新增的 action 在菜单上会是空白。既有先例也都在这边：
  `AssetMenu { id, label }`、`DebugSection { id, title, factory }`。
  **代价**：契约从此携带文案，将来做 i18n 要再挪一次——单语言阶段不值当预付。
- **`checked` 是必须的**：两个 configure 是开关，菜单要渲染勾选态，而当前值
  只有服务知道。
- **`args` 不进描述符**：`Payload` 无 `PartialEq`/`Default`，进了 `ActionDesc`
  就进 `TreeSnapshot`，三个 derive 全断、3 处 `TreeSnapshot::default()` 要改。
  挂在 `ActionInvoke` 上则**一个 derive 都不掉**（消息构造一次即弃，不进快照）。

### D4: `ActionInvoke.node` 是 `Option<String>`，目标单源

- **为什么 `Option`**：`create` 是 workspace 级动作，没有目标节点——而 (D5)
  已让服务持有 `workspace_dir`，它根本不需要目标。`None` 是语义真实的空。
  合成一个 `workspace://` id 会与服务已有状态完全冗余，纯仪式。
- **为什么目标不放 `args`**：`ConfigureReq` 若同时带 `solution_path` 与
  `node`，同一个事实两个来源，正是本 change 在拆的双源。退掉
  `solution_path` 后"作用于谁"只有 `node` 一处说。
- **开关的值由视图在点击时构造**（实施期修订，与 2.5 构造 `CreateReq`
  同形 —— 视图构造 typed `args` 本就是本设计的既定做法）：视图读描述符的
  `checked` 取反，构造 `ConfigureReq` 作为 `args`。
  **"不预构建"仍然成立**：预构建的 `Payload` 会把快照期的值冻结（D3 已否
  这条路）；点击时取反则状态若已变、同值重放恰好落进 `configure_is_noop`
  的 no-op，天然无害。
  （原结论句“无参开关不走 args、服务现算翻转值”已修订 —— 那会让 spec
  的 `Re-applying the same configuration is a no-op` 场景变成死场景。）
- **解析失败的归属**（默认，未单独确认）：node 缺失或前缀不对 = 接收方
  契约 bug，走 `reject` 落死信（与 `Payload::downcast` 失败的 `TypeMismatch`
  同类语义）；格式对但 solution 不存在 = `OpError::NoSuch`（用户可见红条）。
  不新增 `OpError` 变体。

### D5: 服务持有 `workspace_dir`，`LoadReq` = 绑定

`SolutionService` 状态从 `items` 变 `items + workspace_dir`，在 `on_load`
成功时写入；load 失败不更新（`Err` 分支不碰，正好对齐 spec"失败不覆盖既有
状态"）。`CreateReq` / `ConfigureReq` 各自退掉 `workspace_dir` 字段。

- **为什么不是每个请求继续带**：今天每请求都带是"请求只有三个"的巧合，不是
  设计；加了通用 invoke 之后，(a) 会让契约里每个 action 都拖一个与 action
  语义无关的字段，而服务明明已持有该域真值。方向与 D7"状态单份归属服务端"一致。
- **不顺手瘦身 `LoadReq`**：它就是绑定动作本身，`workspace_dir` 是载荷。

### D6: 写入口收敛为 `solution.action` 单点

`KIND_CREATE` / `KIND_CONFIGURE` 退场，`dispatch` 变 `match action_id`。

- **为什么不是两套并存**：契约里同时存在"按 kind 分"和"按 action_id 分"
  两套分发，就是本 change 要拆的双源。乙的直接后果。
- **代价**：边界上放弃 per-action 类型安全——所有 action 共用一个消息形状，
  类型安全移到服务内部 `match`。这是对的：插件 DLL 没法往宿主 enum 里加变体。
- **未知 id 的归属（实施期定案 C）**：`node` 格式坏 = 真的无法解释目标 →
  `reject` 落死信（`TypeMismatch` 同类）；`action_id` 未声明 = 消息看得懂、
  只是操作不支持 → **立即回 `OpError::UnknownAction`**。
  分开的理由：后者是用户真会点到的路径，走 reject 要等 `DEFAULT_CALL_TIMEOUT
  = 5s` 才拿到 `RouteError::Timeout`，且 notice 只写 "ask timeout" ——既慢又
  不点名。原设计“未知 id 一律 reject”已修订。

### D7: 回执路由键从 UI 文案改为 `action_id`

今天 `pending_sol_ops: HashMap<Corr, &'static str>` 的 label **身兼两职**：
既进 `format!("{label}失败: {e}")` 做显示，又被
`sol_oks.contains(&"添加 solution")` 和 `n.contains("添加 solution")`
（`app.rs:1027`/`1036`）**当控制流**——表单签要不要自关取决于错误字符串里
有没有那四个字，改文案就断。

- **为什么乙 逼出了它**：label 今天是 `&'static str`（调用点写死的字面量），
  乙之后来自 `ActionDesc.label: String`（服务注册、运行时才知道）——
  **`&'static str` 必然放宽**，这些行本来就要改。同批换掉路由键，边际成本
  接近零；不换则是把已知的坑原样搬进新契约。
- **依据**：agent 侧已经是这个形态——`pending_ops: HashMap<Corr, (CtrlOp,
  &'static str)>` 按枚举分派、`successes: Vec<CtrlOp>` 是枚举不是字符串。
  solution 侧是唯一没对齐的一处。
- **`n.contains` 那条尤其危险**：失败路径上靠子串匹配决定要不要把错误写进
  表单签，文案一改错误就静默漏掉。
- **成本**：`take_notices()` 一个 `Vec<String>` 同时供"顶栏 notice"与"表单签
  行内错误"两个消费方，结构化后要么拆两条通道、要么 notice 元组化。估 2-3 个读点。

### D8: 改名在 spec 层全量跟进（甲）

11 个 requirement 块正文改名 + `Sidebar session item display` 走 RENAMED +
`agent-shell`/`dock-layout` 的 Purpose 直接编辑主 spec。

- **RENAMED 的坑**：archive 只接受 **RENAMED + MODIFIED 用旧 header** 的组合；
  用新 header 会被判 "not found" 而拒绝归档（实测）。
- **场景名不可改**：MODIFIED 块的场景名必须与现行 spec 逐字一致，否则
  archive 拒绝——所以 `Sidebar selection grants…`、`Drag widens the sidebar`
  这类场景名里的 "sidebar" 保留原样。这是工具约束，不是疏漏。
- **跨 change 不重叠**：`dock-layout` 的 `Left dock is not closable` 归
  `launcher-startup`，本 change 只动 `Left dock draggable resize`；两个
  `project-model` requirement 也分属两 change。**须按 A → B 顺序归档**，
  否则 delta 叠加时基线漂移。

## Risks / Trade-offs

- **[契约携带文案 = i18n 迟早要再搬一次]** → 单语言阶段接受；记入
  `pending-decisions` 类的后评估条目。
- **[通用 invoke 放弃边界类型安全]** → 未知 id 落死信，与 `TypeMismatch`
  同语义；服务内部 `match` 保类型安全。
- **[30 处测试读点改写]** → 生产代码只 4 处；改写是机械的，但要防漏——
  用 `grep -rn "\.tree\b\|\.handoff\b"` 收尾核对。
- **[文案路由改 action_id 漏改一处]** → `app.rs:1027`/`1036` 两处必须同时
  改，否则表单签自关或行内错误会失灵；配断言锁住。
- **[改名波及全部 sidebar 锚点断言]** → `test_support` 锚点（`sidebar-root`、
  `workspace-switch` 等）随模块改名同步，测试断言一并更新。
- **[solution 与 project 同名双行可能被用户当成渲染 bug]** → 类型标记 +
  tooltip 完整 path 是缓解；若反馈仍差，备选是 v1 隐藏 solution 层（记入
  后评估）。

## Migration Plan

单 change 内完成。建议实施顺序：先 D1/D5（契约形状）→ D3/D4/D6（消息机制）
→ D7（路由键）→ D2（视图行）→ D8（改名收尾），因为改名波及面最大、放最后
可避免与前面的结构改动互相踩。归档须在 `launcher-startup` 之后。

## Open Questions

- **③ asset-view 是否共用 `ActionDesc` 类型**（用户保留，自看现状）：若共用，
  类型应从 `solution-service::contract` 上移到 `contracts::shared`（中立、
  零依赖，正是它的既定用途）；若不共用，`AssetMenu` 维持原样。**不影响本
  change 的任务拆分**——`node-actions` capability 已按跨面板机制写，后续接入
  不需要改本轮 spec。
- **④ Effect 回执形状**（用户保留，自看现状）：推演结论是 **solution 侧本轮
  用不上**——configure/remove/create-worktree 全是状态变更，回执就是既有
  `OpResult`（带新快照）。Effect 词汇（剪贴板/reveal 要窗口上下文）的真实
  需求在 asset 侧，属 ③ 的地盘。若 ③ 落在共用，④ 才需要在 `node-actions`
  里补一条回执 requirement。
- **worktree 层级与 `create worktree` 的参数表单**：本轮只留层级占位与
  `args` 通道，实际落地时需补 spec。
