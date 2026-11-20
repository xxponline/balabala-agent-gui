# Design

## Context

动机见 proposal.md（Why）。此处只记塑造方案的现状约束：

- **实例表无契约身份**：`ServiceDescriptor = {name, origin, meta}`、
  `ServiceMeta = {description}`；`Instances` 按 `InstanceId` 存
  `Participant{id, name, state}`，同名可 N 个（`instances_of`/`pick_available`）。
- **寻址带选择策略**：`Route::Service(name, Direct|Any)`；生产调用 10 处全为
  `Any` 且全指向 "agent" 单例，`Direct` 生产零使用（只在定义/测试里）。
- **fingerprint 不是类判据**：它是"rustc 版本 + 全部契约源码树哈希"，
  所有服务同值，分不出类。
- **TypeId 稳定性已是既有支柱**：payload `Arc<dyn Any>` 跨 dylib downcast
  每天在工作——零依赖契约 crate 被宿主与插件共享同一份 rlib，
  同 def-path ⇒ 同 TypeId。本设计复用同一前提，且只在**宿主侧**铸造比较。
- **依赖形状**：`contracts` 零依赖（不能引用框架类型）；
  `service-manager` 不依赖 `contracts`（框架对契约内容保持通用）；
  指纹符号机制（`FINGERPRINT_SYMBOL` + 脚手架生成文件）是现成的
  "产物导出固定符号、装载期读取、缺失显式拒载"先例。
- **容器先例已跑通**：`agent` 服务 = 骨架单例 + 内部 N 会话 + 投影消息
  （`SessionsChanged{SessionView}`）上报子状态 + ask/tell 控制。

## Goals / Non-Goals

**Goals:**

- 骨架内"契约 → 恰一实例"，装载期强拒载；契约身份显式、可解析、缺失即拒。
- 寻址契约面去掉"挑选"词汇：实例直呼 / 服务名直呼唯一实例。
- 容器形态零框架代码（约定 + 既有先例），子实例永不进骨架表。
- 框架其余六条规格行为（信封/ask-tell/fail-fast/邮箱/监督/事件）零变化。

**Non-Goals:**

- 不实现任何负载均衡/分客户机制（容器的私有事务，框架不预设策略）。
- 不动 fingerprint 覆盖范围（见 Open Questions）、不动 `contracts` crate
  的目录组织、不改 `InstanceId` 分配与状态机语义。
- 不做进程级/跨进程容器（容器 = 进程内普通服务）。

## Decisions

### D1 类判据 = 宿主铸造的 `ContractId`（`TypeId` 新类型）

`service-core` 新增 `ContractId(std::any::TypeId)`（Copy/Eq/Hash/Debug），
值一律由**宿主**用契约 marker 类型 `TypeId::of::<T>()` 铸造。

- 替代 (a)：类 = 服务名——被否（名字是别名洞，同契约两名可绕开单例；
  用户明确选 TypeId）。
- 替代 (b)：类 = fingerprint 哈希——被否（全体同值，分不出类）。
- 替代 (c)：跨 dylib 传 TypeId 数值——被否（`TypeId` 无稳定序列化出口；
  宿主单点铸造则根本没有跨边界问题）。

### D2 契约名 → `ContractId` 的注册表归宿主所有

- 内部服务：`register_internal(name, factory, contract: ContractId)` 显式
  携带（编译期 marker，类型安全）。
- DLL：产物新增固定导出符号（契约名字符串，仿指纹符号，脚手架生成）；
  `load()` 装载期读符号 → 查**宿主注册表**
  `register_contract(名, ContractId)` → 缺符号/未知名 → 显式拒载
  （复用 `MissingSymbol` 变体 + 新增 `UnknownContract` 变体）。
- 替代 (a)：注册表放 `contracts` crate——被否（零依赖，不能引用
  `ContractId`）。替代 (b)：`service-manager` 依赖 `contracts` 写死——被否
  （框架退化为单契约专用，破坏"契约即插件"的通用性）。
- 注册表同名不同 marker = 宿主引导期编程错误 → debug 断言。

### D3 删除 `SelectPolicy`，`Route::Service(name)` 单形态 ⚠ 待复核假设

生产 10 处 `Any` 全是"直呼 agent 单例"，`Direct` 生产零使用；单实例下
"挑哪个"是永远没有答案的伪问题，保留即契约面噪音。改动 = 契约枚举 1 处 +
router 分支收敛 + ~20 处机械调用点 + 规格 req2（已按此起草）。

- 替代：保留 `Direct|Any` 语义退化——零改动，但把死词汇永久留在契约面，
  与"更清晰"目标相悖。

⚠ **假设声明**：探索最后一问（"SelectPolicy 选保留还是删除"）用户未逐字
答复即进入 propose；本设计按推荐方案（删除）成文，proposal 同处标注。
**实现启动前必须向用户复核此条**；若改回保留，规格 req2 delta、D5、
对应任务需回退为"语义退化"版本（改动量更小，不阻塞其余结构）。

### D4 拒载门 = `load()` 建实例之前，双保险落表

`load()` 流程：解析契约身份（内部自 descriptor；DLL 读符号+查表）→
查 `by_contract` 索引占用 → 占用即 `ContractAlreadyLoaded {loaded_name,
requested_name}`（**任何状态都占位**，含 `error`——调用方走既有
`retry()`/删除释放槽位）→ 未占用则 create。`Instances::insert` 维护
`by_contract` 索引并断言唯一（防御层）。同名二次装载与同契约异名装载
共用同一条检查路径。Catalog 注册本身不设门（两名同契约可并存于目录，
只挡装载）。

### D5 服务名解析收敛为唯一实例查找

`Instances` 保留按名查找（单实例后自然 1:1）；router 名寻址分支 =
按名取唯一实例 → 复用既有 fail-fast/状态检查；删 `Direct` 归属断言分支
（实例存在性与状态检查已覆盖其防御价值）。`Route::Instance`、`Address`
不变式、`corr` 应答全部原样。

### D6 容器 = 约定，不加一等公民机制

框架不新增 `kind: Container`、不提供子实例注册表/轮询工具箱。容器型服务 =
普通服务内部私有多路复用：子实例不是 `Participant`、不进实例表、不受
骨架唯一性/状态机管辖；子状态经**该服务自己的投影消息**上报（先例：
`SessionsChanged{SessionView}`），子故障与子生命周期由容器自行处置，
容器自身崩溃走既有 supervisor 单实例 `error` 路径。

- 替代：框架一等公民（descriptor 加 kind + 容器工具箱）——被否（约定派
  已被 agent-service 验证；容器怎么均衡/粘客户是业务策略，框架预设即
  越界；等 ≥3 个容器出现重复骨架再议）。

### D7 错误面

- `LoadError::ContractAlreadyLoaded { loaded_name, requested_name }`——
  不携带 `ContractId`（`TypeId` 无 `Display`），两个服务名即足够用户
  可读的"被谁占着"。
- `LoadError::UnknownContract { name, path }`——DLL 声明的契约名不在宿主
  注册表。缺符号复用既有 `MissingSymbol`（`symbol` 字段传契约符号名）。

## Risks / Trade-offs

- **[service-core 枚举形状变更 ⇒ 既有插件 dylib ABI 失配]**：fingerprint
  只覆盖 rustc+契约源码树，**不覆盖 service-core 源码（既有盲区）**——
  旧插件可能指纹通过但与新 `Route` 布局失配。缓解：本 change 内全量重编
  插件产物（flow_tests 天然生成新产物、example-plugin 随 workspace 重建）；
  盲区本身列为后续独立 change 候选（见 Open Questions）。
- **[D3 假设未获显式确认]** → 实现启动前向用户复核（tasks 首任务即此门）；
  改回保留方案的回退面已评估：规格 D5/req2 换回轻改版，结构决策不受影响。
- **[同契约异名强语义封死热备/灰度]** → 已由用户确认接受；替代路径 =
  容器内私有双子（对外仍是唯一实例）。
- **[宿主注册表手工登记漂移]**（DLL 声明名与宿主表不一致 → 拒载）→
  显式错误即暴露手段，与指纹"绝不静默"同哲学。
- **[error 占位需人工释放]** → 错误携带占用方名字，GUI/日志可直接提示；
  这是监督语义（supervisor-only recovery）的自然延伸，不加自动恢复。
- **[测试重写]**：`task_3_3_multi_instance_internal_service`、
  `instances_of.len()==2`、`pick_available` 族、路由 Direct/Any 用例 →
  改写为拒载/解析语义；基线 253 测只增不减（拒载场景 ≥3 个新增）。

## Migration Plan

单进程内存态，无持久数据迁移。实现序（每段全绿后推进）：

1. `ContractId` + marker + 宿主注册表 + descriptor/Participant 携带；
2. `load()` 拒载门 + 新错误 + `by_contract` 索引 + 符号解析；
3. `Route` 单形态 + router 收敛 + 调用点机械改写（D3 复核通过后）；
4. 测试改写/新增 + 插件产物全量重编 + `cargo test/clippy` 全绿。

回滚 = 整体 revert（契约面变更与 spec delta 同进同退）。

## Open Questions

- **fingerprint 覆盖范围**：是否应把 service-core（及框架 crate）源码哈希
  纳入指纹，堵住"框架枚举变更、旧插件指纹仍过"的盲区？不阻塞本 change
  （本 change 靠全量重编兜底），建议独立小 change。
- **D3 复核结果**（非可延后项——已提级为实现首任务的决策门）。
