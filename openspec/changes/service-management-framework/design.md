# Design

## Context

动机见 proposal.md - Why。此处只列约束现状：

- 单 package 工程（`balabala-agent-gui`），gpui-kit GUI；异步全部走 gpui
  `BackgroundExecutor`，**进程内目前零 tokio**。
- `kameo = "0.22.2"` 已入 Cargo.toml 但 src 零引用 —— 本 change 起实际使用；
  kameo 绑定 tokio，运行时拓扑因此是硬约束（D9）。
- 现有可复用先例：
  - `asset_manager` 的"扫描 → 异步枚举 → 落状态"骨架（Catalog/Scanner 形状
    参照它）；
  - `Docs/module-split-design.md` §7.2 实测结论：**展示方必须被显式 notify，
    自读不重建** —— GUI 投影模式（D10）直接复用这条已趟平的坑。
- 终端输出现状 = 每 session 独立线程 + 共享 `AnsiBuffer` + 宿主 50ms 轮询 +
  显式 notify（`Docs/pending-decisions.md`）—— 是 D11 带外逃生门的既有形态。
- 用户已确认的边界前提：服务分**内部 crate** 与**外围 DLL** 两类，**基本一起
  编译**（无兼容性负担）；DLL 只做工程隔离，将来接第三方再单独做 Bridge。

## Goals / Non-Goals

**Goals:**

- 中间层三件套（core/manager/plugin）可独立编译、独立被引用，对 GUI 与具体
  服务契约零依赖，未来可整体搬为独立仓库。
- 五条需求落地：扫描/装载（多实例）、标准状态机、统一路由互通、内部与 DLL
  服务同一契约同一路径、GUI 成为框架的普通参与者。
- 为后续模块迁移（AgentEntity、AssetManager 等 → 服务）铺好骨架，但本
  change **不迁移任何现有模块**。

**Non-Goals:**

- 不做文件系统 watch / 热插拔 / 进程内热换代。
- 不做服务进程隔离、网络远程路由（kameo remote 暂不启用）。
- 不做第三方插件兼容（Bridge 是将来单独的 change）。
- 不改任何现有 spec 的需求与 GUI 行为。
- 不实现数据面带外通道（仅在 D7 留逃生门位置）。

## Decisions

### D1 演员位置与边界立场：插件侧 kameo actor + 宿主侧 actor，边界只传契约类型

- 插件内的服务本体是 kameo actor；宿主自带的 actor（含将来 GUI 侧服务）同样
  注册进同一套框架，**路由表里没有特权参与者**。
- 边界立场：一起编译 ⇒ 不预付兼容成本；契约 crate 的类型可直接过边界。
  将来接第三方：在插件侧加 Bridge 把契约降级为 C-ABI + 序列化，信封与 Router
  不动。
- 备选（否决）：边界一律 C-ABI + bytes —— 在"一起编译"前提下是提前付费；
  边界一律共享 tokio actor 类型 —— 耦合运行时细节，且仍绕不开 ABI 问题。

### D2 契约结构：单 contracts crate，按服务分目录，预留拆分形状

- 全部服务的 Message 类型放一个 `contracts` crate，内部按服务分命名空间
  （`contracts::agent`、`contracts::asset` …）；将来通信图成形后可机械裂出
  `contract-agent` 等独立 crate。
- 消息类型归属**调用方**（caller 拥有自己发的消息与期待的回复类型，放
  自己服务的契约命名空间），被调方 import 调用方契约；**双向共享的消息
  类型归中立命名空间**（`contracts::shared`，裂分时 = 独立中立 crate，
  零依赖）。这是裂分无循环的前提：若归被调方，双向通信会在裂出
  `contract-a`/`contract-b` 后形成编译期互依。
- Router（`service-manager`）**不依赖 contracts**：信封对 payload 不解释，
  中间层与具体服务解耦，这是"独立可引用"的结构保证。
- 备选（否决）：一服务一契约 crate 起步 —— 沟通图未画出，按想象分包会分错；
  分收益（增量编译/依赖可视化）在"一起编译"下不兑现。

### D3 路由拓扑：中心枢纽（方案 A），全部消息过 Router

- 宿主 actor、GUI、插件 actor 的一切通信（含 ask 的回复）都走统一信封过
  中心 Router。收益：观测面完整（谁跟谁说话、延迟、超时率）、死信有去处、
  状态与消息流共用一个观测点、宿主/插件/ GUI 统一待遇。
- 备选（否决）：查表 + 直连（kameo ActorRef 直递给对方，类型化零中介）——
  数据面最热，但服务间形成编译期耦合、ActorRef 跨 dylib 边界踩 D12 的雷、
  GUI 旁观流量要另造机制；作为将来热路径逃生门保留（见 D7），不在 v1 做。

### D4 寻址：两级 —— `Route::Instance(..)` 与 `Route::Service(name, policy)`

- 实例直呼：调用方拿着 `load()` 返回的实例 id 精确投递。
- 服务名寻址：Router 挑实例，策略 v1 只有 `Direct(id)`（携带实例 id）与
  `Any`（挑一个 available）。`Direct(id)` 与 `Route::Instance(id)` 的区别：
  `Instance` 是裸实例地址；`Service(name, Direct(id))` 多一层"该实例必须
  属于该服务"的断言，防止实例 id 用错服务名时误投。`Any` 是唯一
  "不知道目标是谁"的入口。round-robin/亲和性等真有多实例负载需求再加。
- 备选（否决）：只认实例地址 —— "给我找个活着的 X"会在每个调用方重复实现；
  只认服务名 —— 多实例（多 session）场景调用方明明知道目标是谁。

### D5 信封与 payload：类型化 Arc 默认，序列化是显式消息类型

```rust
Envelope { to: Route, from: Route, corr: Option<Id>, kind: &'static str,
           payload: Payload }
enum Payload {
    Typed(Arc<dyn Any + Send + Sync>),  // 默认: 共享 Message, 零序列化透传
    // 特殊封装 (JSON/bincode) 不是 Router 的分支 —— 它本身就是一个
    // Message 类型 (如 Json(String)), 应用层自取所需
}
```

- Router 只搬 Arc、只认 route/kind/元数据；`kind` 是手写标签（日志/观测用），
  **不依赖 TypeId**（见 D12）。
- `from` 只允许实例/actor 地址，不允许 `Service(..)` 歧义地址——回复与回报
  一律按 `corr` 在 Router pending 表销账，**不按 from 寻址**；`from` 纯粹
  用于观测（"谁发的"）。
- `kind: &'static str` 若来自插件，指向插件 .rodata，其安全性与 D8 的
  "永不 dlclose" 承诺绑定（真 dlclose 即悬垂指针）。
- 接收方对 `Typed` payload downcast 失败（与 `kind` 约定的类型对不上）=
  接收侧拒收，落死信并以 `TypeMismatch` 观测——这是接收方契约 bug，不是
  路由错误。
- 大块数据的变更通知**自带快照 Arc**："changed" 事件直接携带新快照的
  `Arc`（payload 本来就 Arc 透传），接收方 `snapshot()` = mailbox 最后一值，
  零回程 ask；按需查询才用 corr ask。
- 类型化让"过总线的税"降到 channel hop + Arc clone 级（纳秒~微秒），因此
  大块数据（整棵资产树等）**原则上也过总线**，机制保持单一。

### D6 ask 语义：回复走总线、Router 持超时、fail-fast 零缓冲

- call = envelope(corr) 进总线 + 本地 oneshot；回复 envelope 带 corr 原路
  返回 Router 销账 —— 一问一答全程留痕。Router 不阻塞：查表 → 放入
  per-outlet 有界通道（按目标 FIFO）→ 记 pending{corr, deadline}。
- **per-outlet 有界通道满 = 投递失败，不是隐式缓冲**：Router 用 try_send
  入队，满即刻销账——ask 收 `Err(DeadLetter(ChannelFull))`，tell 落死信，
  调用方不会挂在 pending 表干等超时。有界通道只是目标侧 FIFO 整形；
  fail-fast 的"零缓冲"指不为目标非 available / 拥塞保留任何隐藏队列。
- ask 的收口按参与者形态分两种，同一张 pending 表：runtime 驱动的参与者
  经本地 oneshot（future 门）；宿主驱动的参与者（GUI）`submit(corr)` 后
  回复落自己的 mailbox，等 poll 取件。
- 超时由 Router 持有：到期销账、落死信、调用方收 `Err(Timeout)` —— 超时是
  可观测事件，不是调用方默默丢回复。
- **fail-fast**：目标非 `available`（loading/offline/error）时立即拒绝 ——
  ask 收 `Err(TargetNotAvailable(state))`，tell 落死信；Router 无任何隐藏
  缓冲。等待可用 = 调用方订阅状态翻转后重发；横向弹性靠 `Any` 策略
  （这不行换另一个活着的）。
  - 备选（否决）：loading 期隐式排队 —— 为毫秒级窗口引入永久缓冲机制，
    offline 排队更是替用户猜测意图；失败与等待显式长在调用方，GUI 可见
    中间态。
- **tell 的无声失败是认账的**：tell 无同步反馈，落死信只有观测面可见。
  因此约定：**GUI 用户触发的操作一律走 ask**（错误可回显给用户）；tell
  只用于 fire-and-forget 的内部流（事件广播、状态推送）。
- 调用方 API：`Result<Reply, RouteError>`，错误至少含 `Timeout` /
  `TargetNotAvailable(state)` / `DeadLetter(reason)` / `NoSuchRoute` /
  `TypeMismatch`（回复 payload downcast 失败）。

### D7 每实例状态机 + 观测

```
  (不存在) --load()--> loading --init ok--> available
                          |                 |        |
                    init 失败           disable()   crash
                          v                 v        v
                        error             offline   error
                          |                 |        |
                          |        enable() |        | retry()
                          +-----> loading <-+--------+
                          |
                       删实例 --> (不存在)   [仅 error 可删]
```

- **状态挂每实例**（supervisor 持有），不是全局；`offline = 主动停用`
  （可 enable），`error = 起不来/跑挂了`（可 retry 或删），**未 load 的服务
  不存在于实例表**（三者互不相同）。
- **恢复策略单一权威 = supervisor 状态机**：kameo 的 supervision/自动
  restart **禁用**——actor crash 只上报不自动拉起，supervisor 落 `error`，
  重建只能经显式 `retry()`（或 enable()）走 loading。两层恢复不允许并存：
  否则"重启窗口内消息全拒（fail-fast）"由谁负责、算什么状态，无定义。
- 状态翻转以事件推送宿主（通道）；**订阅塌缩成路由**（= 往路由表加一条
  "该实例的翻转事件也投到我地址"），GUI 从 mailbox 取件后投影（D10）。
- 数据面带外（共享缓冲/直连）暂不做；若将来 profile 说 channel hop 过热，
  逃生门 = 在 Router 登记带外通道（观测保留"谁跟谁有通道 + 字节计数"，
  不看每帧内容）。

### D8 Catalog：启动静态扫描 + 手动 reload，无 watch；句柄承诺

- 启动扫一遍服务目录建 Catalog；提供显式 `reload_catalog()`（GUI 按钮/测试
  调用）。文件系统 watch 被否决：插件产物时机由自己控制（一起编译），
  watch 会让 Catalog 变成持续变化的共享状态，三处应付条目蒸发。
- `reload_catalog()` **只影响未来的 load，不动实例表**：已 load 实例继续跑
  （Library 句柄已 Arc 持有），条目消失只意味着不能再 load 新实例；同名/
  产物替换的合并规则见 Open Questions。
- **构建指纹校验（D12 路线甲的防线）**：每个 cdylib 以 build.rs 嵌入契约
  构建指纹（rustc 版本 + 契约 crate 元数据哈希）并导出为固定符号；Loader
  在 create 实例前比对宿主指纹，不匹配**拒载**（条目落 error/扫描警告）。
  TypeId 撕裂最常见的触发路径是"插件单独构建后 reload 换入"，指纹校验
  把它从 downcast 静默失败变成显式拒载。
- Catalog 条目 = `ServiceDescriptor { name, origin: Internal(fn) | Dll(path),
  meta }`，内部服务启动时显式 register、DLL 经扫描，**进同一张表**，
  load 流程只按 origin 分支（唯一区别是符号从哪来）。
- 同一 DLL 多实例 = 1 次 dlopen + N 次 create；Library 句柄 `Arc` 共享，
  与实例表同寿。
- **`unload()` 的措辞必须诚实**：停用并移除实例，但**不 dlclose**
  （热 dlclose 有 UB 风险，地址空间也不缺那几 MB）；真回收 = 进程重启。
  API 文档不得暗示可回收动态库。

### D9 运行时拓扑：中间层自建自持 tokio，对外不泄漏 tokio

```
GUI 主线程 (gpui, 非 tokio, 永不阻塞)
   |-- load/call/subscribe 非阻塞 --> [中间层 runtime: tokio multi-thread]
   |                                    Router/supervisor/actor/ingress/
   |                                    定时器/转发循环 全在这里
   <-- 事件通道 / 回调 (状态翻转、call 完成) --|
```

- `call()` 实现：内部**急切 spawn** 到 layer runtime（不等 poll）→ 内部
  随便用 kameo ask / tokio::time → 结果进 oneshot → 对外返回的 future
  **只剩 oneshot await**，executor-agnostic：gpui executor await 得动、
  tokio await 得动、测试 block_on 得动。
- **双门 = 两种消息循环宿主形态，无回调门**：runtime 驱动的参与者（服务/
  测试）走 async 门（future）；宿主驱动的参与者（GUI）走 **poll 门**——
  **GUI 就是总线上的拉模式 actor**：有自己的 Route、有自己的 mailbox
  （Router per-outlet 队列），`Listener` = mailbox 句柄，`poll()` = 一次
  消息循环迭代（gpui 泵驱动，GUI 主线程同步执行）。API 面 = `submit(corr)`
  /`poll()/drain()/snapshot()`，**不提供任何回调**，跨线程回调合同问题从
  结构上消失（曾评估"回调门 + poll 派发"的糖，否决：API 面最小即 B）。
- **订阅塌缩成路由**：`subscribe_state(id)` = 往路由表加一条"该实例的
  状态翻转事件也投到我地址"，不是独立推送机制。
- "对外不泄漏 tokio"的边界是**宿主/GUI**：service-plugin 脚手架以 kameo
  actor 形态提供，kameo/tokio 类型对**插件作者**可见（他们本就在 runtime
  里）；宿主双门 API 中不出现任何 tokio/kameo 类型。
- 备选（否决）：宿主提供 runtime（注入 handle）—— 宿主今天没有 tokio，
  让宿主建运行时违背"宿主零侵入"；将来宿主真需要 tokio 时再加
  `with_runtime()` 注入口，不影响现有形状。

### D10 GUI = 投影者

- GUI 实体不再拥有数据，只持"订阅 + 本地快照"：

```
service 状态/数据翻转 --事件(自带快照 Arc)--> GUI 泵(listener.poll) --cx.defer--> entity.notify() --> render
                                   |-> snapshot() = mailbox 最后一值, 零回程
```

- 渲染是同步的、actor 是异步的 → render 只读本地投影，零 await；这与
  module-split-design §7.2 实测的"显式 notify 才重绘"完全同构，坑已趟平。
- 投影所有权放 **GUI 侧**（view entity 自己缓存），manager 不替 GUI 存
  展示状态。

### D11 工程结构与依赖方向

```
balabala-agent-gui/  (workspace)
├── crates/
│   ├── service-core/      # Route/Envelope/Payload/ServiceState/RouteError
│   ├── service-manager/   # Router/Scanner/Loader/Supervisor/自建 runtime
│   ├── service-plugin/    # ingress/actor 脚手架/extern 入口
│   ├── contracts/         # 全部服务 Message (按服务分目录, 零依赖)
│   └── (agent-service/asset-service/... 后续 change)
├── plugins/               # cdylib 产物落点 (= 扫描目录; 产物 gitignore, 不进版本库)
└── src/                   # GUI 本体, 变薄壳 (本 change 仅接入, 不迁移)

依赖单向: GUI -> contracts + manager; 服务 impl -> contracts + core/manager;
          manager -> core (永不到 contracts/GUI); plugin -> contracts + core
```

- 三件套零上层依赖 → 未来整体搬独立仓库 = git 移目录；本 change 先
  workspace，不预付独立仓库管理成本。
- `Cargo.toml` 单 package → workspace 是 **BREAKING**（构建入口结构变化，
  `cargo build` 行为不变）。

### D12 spike：类型化 payload 跨 cdylib 的 TypeId 一致性

- `Arc<dyn Any>` 的 downcast 依赖两侧 `TypeId` 相等；同次 cargo build 中
  contract rlib 通常只编一次 → 相等；但 feature unification 分叉（插件开
  `contract/serde`、宿主不开）会编两次 → **downcast 运行时静默失败**。
- 两路线：
  - 甲（v1 起步）：依赖"契约只编一次"（同一 cargo build 内 StableCrateId
    一致 ⇒ TypeId 相等）——用 D8 的**构建指纹校验**让"只编一次"可验证：
    插件单独构建后 reload 换入是最常见撕裂路径，指纹不匹配即拒载；再加
    集成测试钉死（真 dlopen 后 round-trip 一个 Typed payload 断言成功）；
  - 乙（设计目标）：downcast 永远在宿主侧 —— DLL 的 Outlet 是宿主侧 typed
    闭包（契约类型已知），编好后以 typed extern fn 递 `&Msg` 进插件，
    插件侧永不碰 Any/TypeId。
- 甲先行、接口形状为乙预留；甲的测试失败即触发切乙，不改信封与 Router。

## Risks / Trade-offs

- [TypeId 跨 dylib 静默失败] → D12：构建指纹校验（拒载而非静默失败）+
  集成测试钉死 + 乙路线兜底；`kind` 标签不依赖 TypeId，日志/观测不受影响。
- [Router 单点：性能与故障面] → 转发不阻塞（查表+入队即返）、per-outlet
  有界通道隔离堵点、pending 表即观测面；v1 不做多 Router，热路径逃生门
  （D3/D7）预留在案。
- [fail-fast 把重试/等待推给调用方] → 认了这笔显式代价：类型化错误 +
  状态订阅可拼出"转圈等待"且中间态用户可见；核心服务启动期由启动序列
  兜底（见 Open Questions）。
- [contracts 单 crate 全体重编] → "一起编译"前提下代价≈0；拆分形状已
  预留（D2），沟通图成形后再裂。
- [tokio 运行时线程池进入宿主进程] → 自建自持、默认最小线程数；宿主 API
  零 tokio 类型暴露（D9）。
- [unload 不 dlclose 的承诺容易被误读] → API 命名与文档明确"停用"语义
  （D8），不叫"卸载回收"。

## Migration Plan

1. **workspace 化**：根 Cargo.toml 改 workspace，现 package 原位变成员，
   全部现有测试保持绿（纯结构变化，无行为变化）。
2. **三件套 + contracts 落地**（不接 GUI）：core 词汇表 → manager
   （scanner/catalog/loader/supervisor/router/runtime）→ plugin 脚手架 →
   示例插件 cdylib；以**示例插件 + 内部假服务**做集成测试载体（含 D12
   的 dlopen round-trip 测试）。
3. **GUI 接入（本 change 的最后一个消费方动作）**：GUI 启动时装载核心
   假服务/示例服务、订阅状态、提供事件泵；不迁移任何现有模块。
   本 step 的验收边界 = 订阅/mailbox poll 通路可用（GUI = 拉模式参与者，
   listener 接入 gpui 泵）；服务状态页等 UX 形态不在本 change（属后续
   GUI spec change），避免范围溢出。
4. 回滚策略：workspace 化单独一个提交（可独立 revert）；框架三件套对
   现有代码零侵入，回滚 = 移除成员与依赖边，GUI 无行为回退。
5. 后续 change（不在本 change）：AgentEntity/AssetManager/各模块逐个
   服务化迁移、Bridge 第三方插件、specs 补齐（由 `/opsx-propose` 产出）。

## Open Questions

（均可在不改 specs、不改本设计骨架、不改任务拆分粒度的前提下延后决定）

- ~~kameo supervision 的 restart 策略映射~~ **已决（D7）**：supervisor
  状态机唯一权威，禁用 kameo 自动 restart；仅剩实现细节——kameo 0.22 中
  禁用自动拉起的具体 API 形态（on_failure 钩子只上报不重启），不改语义。
- 启动序列失败 UX：核心服务 loading 卡死 / error 时壳显示什么（服务状态页
  的具体形态留给 GUI 相关 spec）。
- `reload_catalog()` 时已存在条目与新扫描结果冲突（同名、产物被替换）的
  合并规则。
- 核心服务清单（哪些 `Direct` 单实例服务启动即载入）—— 模块迁移 change
  逐个落定时自然确定。
- 双门 API 的权重：GUI poll 门（拉模式 actor 的 mailbox）为第一公民已定，
  回调门已否决（B）；async 门的公开程度（是否算对外稳定 API）待有第二个
  宿主时再定。
