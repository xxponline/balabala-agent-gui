# Design

## Context

See proposal.md for motivation. 现状约束（探索期实证）：

- `workspace_property_source`（921 行）一身三职：`WorkspacePropertySource`
  trait + 静态注册表、gitignore 过滤（`filter.rs`）、后台扫描
  （`filetree.rs`：`resolve_active_root` 从活动会话推根、`scan_root`
  排除表、generation/在途守卫）。右槽面板持 `Box<dyn _>` 并自带
  TreeState/展开集/渲染缓存。
- `SourceNodeKind` 被 `property_detail_manager::OpenObject.kind` 引用——
  拆除时必须连带；左键打开（→`open_object` 分发）与右键
  （copy path / reveal）钩子都在旧源体内。
- 领域模型：`Module { name, children }` **没有路径**；`Project { name,
  root, modules }` 的 root 是会话 cwd。file-tree spec 仍以活动会话的工作区
  为根（11 条），dock-layout 仍写 "workspace file tree"。
- 面板已有可平移的机制件：`last_generation` O(1) 失效、`right_dock_open`
  隐藏零开销、展开态 `HashSet<String>` 由面板集合贴回、⟳ 保展开。
- 本 change 的 spec delta 已定行为边界（specs/ 四份，`--strict` 通过），
  本文件只解决 HOW。

## Goals / Non-Goals

**Goals:**

- 数据流单向：注册 → 异步枚举 → 过滤 → view → 面板渲染；面板零数据职责。
- 枚举链完全异步、原子落地（v1 单 Module，与今日扫描同粒度）。
- 旧机制件（排除表、后台线程、generation、在途去重、隐藏零开销）
  **平移不重写**——迁移的是所有权，不是算法。
- 删除 `workspace_property_source` 后不留任何 "source" 概念残骸
  （含 DTO 命名）。

**Non-Goals:**

- 子 Project / Project 引用的渲染与解析（solution 类型扩展时再做，
  spec 已留结构位）。
- 逐 Module 渐进落地（v1 原子；多 Module solution 出现后重评）。
- 打开/右键钩子的最终归属（**用户明确后置**——本 change 暂放
  asset_manager，见 Open Questions）。
- workspace 自身文件的任何管理/展示。
- 过滤规则的按项目覆盖（规则仍是工作区级一份）。

## Decisions

### D1. asset_manager 是 gpui Entity，挂在 TerminalApp 上，面板经 app 读

**Decision**: `TerminalApp` 持 `Entity<AssetManager>`（与 dock_area/tree
并列）；`TreePanel` 保持现有 `WeakEntity<TerminalApp>` 不变，渲染时
`app.read → asset_manager.read → view()`。manager 内部状态
（注册集、view、generation、在途任务）自治，落地后 `cx.notify` +
沿用 app 的 `notify_panels` 转发链。

**Why**: 与项目既有实体模式同构（面板自读 + 显式转发是
spike §10.5 的既定纪律）；Entity 提供主线程串行化与通知，不必新造
Arc<Mutex> + 事件总线。**Alternative rejected**: manager 独立于 app 的
裸服务对象——它的异步落地要打 `cx.notify`，最终还得挂回 gpui 调度，
绕一圈反而多一层锁。

### D2. 枚举契约: `AssetEnumeratable` → `AssetEnumerator`，盒装 Future 承载异步

**Decision**:

```rust
pub trait AssetEnumeratable {
    fn enumerator(&self) -> Box<dyn AssetEnumerator>;
}
pub trait AssetEnumerator: Send {
    fn enumerate(self: Box<Self>)
        -> BoxFuture<'static, Result<AssetNode, String>>;
}
```

- `Project` 实现 `AssetEnumeratable`：枚举器 = 自身快照，异步产出
  `AssetNode::Project`，子节点 = 各 Module 的枚举结果。
- `Module` 实现 `AssetEnumeratable`：从 `root` 构造**标准文件枚举器**
  （独立类型 `FileEnumerator`，`scan_root`/排除表/符号链接策略整段平移
  进它的 future），产出 `AssetNode::Module` + 递归文件子树。
- manager 驱动组合：一次 `spawn` 内 proj.enum() → 逐 module.enum() →
  组装 → 过滤 → **一次 land**（原子，D5）。

**Why**: `async fn` 在 trait 里不满足对象安全（Rust 2024 现状），
`BoxFuture` 是唯一让 `Box<dyn AssetEnumerator>` 可行的形态；`Send` 边界
保证可丢进 `background_executor`。**Alternative rejected**: 关联类型
enum dispatch（`ProjectEnum/FileEnum` 枚举）——把新 solution 类型的扩展
点从"加实现"变成"改枚举"，与注册表纪律相悖。

### D3. DTO 家族改名搬家: `SourceNode/Kind/MenuDesc/SourceEffect`
→ `AssetNode/AssetKind/AssetMenu/AssetEffect`

**Decision**: DTO 全部落 `src/asset_manager/`（`AssetNode{id,label,kind,
children}` 身份语义不变：id 仍是绝对路径字符串——center-tab dedup、
复制路径、`OpenObject` key 全部免改）。`OpenObject.kind` 改持
`AssetKind`；`workspace_property_source` 目录整体删除。

**Why**: "source" 是被拆除的概念，留旧名等于让后来者找不到概念的主人；
`id` 身份体系不变是**关键稳定面**——dedup/菜单/open 的载荷零迁移。
**补充 (实施中钉死)**: Project 行 id = `project://<root>` 命名空间前缀 —
  v1 Project 根与 root Module 根同目录, 裸路径会撞行 id; Project 行无
  菜单/打开语义 (kind=Project 不是 file/directory row), 路径语义仍全由
  Module/文件行的裸绝对路径承载。
**Alternative rejected**: DTO 独立成第三个模块（`asset_dto`）——两个
消费者而已，按"数据随拥有者走"归 manager，少一次跳转。

### D4. 过滤: `Ruleset` 平移进 manager，匹配键 = 相对各 Project 根

**Decision**: `filter.rs` 的 glob 语义（`* ? **`、`!` 重包含、最后匹配胜出、
仅文件行）逐字保留；入参从"strip 工作区根"改为"逐 Project strip 自己的
root"。规则读取仍走 `project_model::load_file_tree_rules(ws_dir)`。
view 落地时对每个 project 子树各跑一次 `filter_tree(node, project_root_id,
&ruleset)`。

**Why**: spec delta 已定"相对 Project 根"；实现上 `filter_tree` 本就收
`root_id` 参数，多项目 = 多调用一次，**算法零改动**。workspace 根概念从
过滤语义退场，正好由 D3 的 id 体系自然承接（绝对路径 id 不受根变化影响）。

### D5. manager 的 view 是三态 + 代数，原子落地

**Decision**:

```rust
enum AssetView {
    Empty,                                  // 无工作区 / 零注册
    Degraded { message: String },           // 模块根不可读 (命名失败)
    Forest { rules: Vec<String>, nodes: Vec<AssetNode> },
}
// manager 内: view + generation: u64 + in_flight: Option<Task<()>>
```

枚举链在一个后台 future 里跑完，末尾主线程一次 swap（view+generation+1）。
刷新/注册变更 → supersedes 在途任务（旧 refresh 覆盖旧任务的既有纪律）；
数据变更通知 → generation 相等即 O(1) 早退（旧 source 的守卫逐条平移）。

**Why**: 原子 land 是 spec 明文（Asynchronous … land as one complete
view），也与今日扫描一致；三态直接映射 spec 的 Empty/Degraded 合同。
**Alternative rejected**: 逐 Module 渐进 land——v1 单 Module 收益为零，
却把面板的"部分树"态变成新状态面。

### D6. 注册接线收敛在 app 的既有两个点

**Decision**:
- `commit_open`（Ready 分支）: `asset_manager.register(ws.model.projects)`
  （内部 clear + 全量 + 触发枚举）；紧邻 legacy 规则补拷——注册前规则就位。
- `apply_add_solution`: 内存 model 合并后 `asset_manager.register_project(p)`。
- 面板 ⟳ → `app.refresh_assets()` → manager supersede 重枚举。
- `resolve_active_root` 及"活动会话"依赖**整体删除**（spec 已把门控改为
  注册集）。

**Why**: 两个点恰好是"项目集合变化"的全部来源（加载/加 solution），
注册逻辑单点可测；切换被 commit 的 clear+register 天然覆盖，无第三条路径。

### D7. 面板瘦身: 数据件全走，显示件保留

**Decision**: `tree/mod.rs` 删掉 `source: Box<dyn _>` 与自身 view 缓存比对
逻辑，改为：读 manager view → generation 相等跳过 → 否则 sync TreeState。
保留：TreeState 订阅、`expanded` 集合（id 体系不变故无缝）、`dir_ids`
图标兜底、渲染缓存、`right_dock_open` 隐藏零开销、⟳/标题/规则摘要/
右键菜单 UI、左键 → `app.open_object` 分发（经 manager 的钩子）。
`set_source` 测试缝 → 注册缝（测试直接 `register` 自己的项目）。

**Why**: spec delta 的分工就是照这个劈的（file-tree 只剩显示 + 交互
触发）；面板现有机制件恰好全是"显示侧"，迁移是删数据件而不是改显示件。

### D8. v1 渲染形状: Project 行与同名 root Module 行并存

**Decision**: view 结构 `AssetNode::Project → [AssetNode::Module] → files`
逐层成行，v1 两行同名（`foo` → `foo`）照出。

**Why**: 结构与模型同构，未来子 Module/子 Project 只是多几行，渲染器
不用特判"root 折叠"；spec（project-model）明文 root module 与项目同名，
折叠它反而藏掉模型事实。**Alternative rejected**: 单模块时把 Module 行
折叠进 Project——省一行的收益抵不过未来引入子模块时的渲染分支。

## Risks / Trade-offs

- [tree/mod.rs 测试大改：source 注入缝变注册缝，约 10+ 个用例改起手式]
  → 注册缝更直白（`manager.register` + 临时目录）；扫描/过滤用例随实现
  平移到 asset_manager 自己的测试模块，断言文本可原样带走。
- [过滤根从 workspace 变 project：同一行规则的命中面改变（外部项目、
  嵌套项目）] → spec 场景 "Patterns are evaluated per project root" 钉死
  新语义；个人工具无存量规则兼容负担，README/pending-decisions 不涉
  （规则文件位置未变）。
- [DTO 改名波及 property_detail_manager / HANDLERS 匹配] → 机械替换
  （grep `SourceNodeKind` 仅 3 处 + 测试），id/key 语义不变，无行为风险。
- [打开/右键钩子暂放 manager 是"临时归属"] → 已在 proposal/design 显式
  标记后置；若归属回调，file-tree 的 MODIFIED 两条措辞要跟着回摆
  （它们写的是 dispatch 经 manager）——回摆成本 = 两个措辞 delta。
- [原子落地在超大项目上仍是"全等才见"] → 与今日行为持平，不回退；
  渐进落地是 Open Question。
- [Entity + notify 转发链若漏一环，面板不刷新] → 沿用
  `notify_panels`（sidebar/terminal/tree 既有名单），无新链路。

## Migration Plan

1. **建** `src/asset_manager/`：DTO 改名入驻 → `Ruleset` 平移 →
   `FileEnumerator`（scan_root 整段）→ manager Entity（注册/枚举驱动/
   view+generation/守卫）→ trait 定义与 Project/Module 实现。
   （此步旧模块仍活，双轨并行，编译绿。）
2. **接** app：`commit_open`/`apply_add_solution`/⟳ 注册钩子。
3. **换** 面板：tree/mod.rs 改吃 manager view；测试缝替换。
4. **删** `workspace_property_source` + `resolve_active_root` +
   `OpenObject` 旧 kind 引用；全仓 grep `Source` 清零。
5. **验**：`cargo test` 全绿（含平移后的扫描/过滤套件）、
   `openspec validate --strict`、`cargo run` 冒烟（注册→树出现→⟳→
   过滤→右键/左键）。
- **回滚**：单 change 纯 git revert；specs 未归档前主 spec 未动。

## Open Questions

- **打开/右键钩子的最终归属**（用户明确后置）：若回调面板或独立
  "打开服务"，file-tree 的 Left-click/Right-click 两条与本设计 D7 的
  "经 manager 分发"措辞同步回摆。
- **逐 Module 渐进落地**：多 Module solution 出现后，view 是否允许
  "部分森林 + 完成标记"（会新增一个 spec 状态面）。
- **LSP 服务接入点**：与 asset_manager 同层的服务未来是消费 manager 的
  Module 集，还是各自独立注册——等 LSP 立项时定。
- **规则的按项目覆盖**：工作区级一份规则外，是否要 per-project 覆盖段
  （`file_tree` schema 扩展，spec delta 届时再开）。
