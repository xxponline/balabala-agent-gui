# Design

## Context

总线骨架（`service-management-framework`）已归档：`ServiceManager`（`register_internal`/`load`/`new_mailbox`/`subscribe_state`）、中心 `Router`（两级寻址、fail-fast 零缓冲、ask 持超时）、`Listener` 拉模式门（poll/drain/snapshot/submit）、Supervisor 状态机全部可用，且**宿主参与者恒 `Available`**（`Participant::host`）、ask 默认超时 5s。GUI 已是普通参与者（`src/service_layer.rs`，50ms 泵 `pump()`，task 6.1 模式）。

会话侧现状（本次要换底的对象）：
- `Workspace::agents: Vec<Entity<AgentSession>>`，`AgentSession(pub Box<dyn AgentEntity>)` 包在 gpui `Entity` 里；
- `AgentEntity` trait（`src/agent_entities/mod.rs`）是大杂烩：控制面（`send_*`/`shutdown`/`resize`/`set_theme`）与渲染面（`screen()`/`poll()` 喂 vt100）揉在一起；
- `src/pane/mod.rs`（2259 行）的 `TerminalPanel` 直接握 `WeakEntity<TerminalApp>`：`active_session()`/`send_key`/`kick_input_drain`/`notify_panels` 全横穿 app↔pane；
- 输入快路径：宿主 50ms 泵统一 drain + 输入后 8ms×4 burst（`kick_input_drain`）；
- 解析发生在 **UI 线程的泵里**（`app.poll_sessions` → `session.poll()` → `parser.feed`）。

既定边界（探索定案，见 proposal）：左槽功能与 spec 原样只换控制底；右槽不涉 agent；框架 crate 零改动；远期原生 RPC 渲染记于 `Docs/future-agent-terminal-native-rpc.md`，近期不做。

## Goals / Non-Goals

**Goals:**
- 第一个真实领域服务落总线：`AgentService` 进程内薄服务 + `AgentTerminal` 纯渲染层，确立"服务管域、UI 发/收消息"的迁移模板（后续 AssetManager 等沿用）；
- 全链路 message 化：控制 ask、输入 tell、帧全推（所有权移动零拷贝）、投影事件快照；
- **行为保持**：既有 spec 全绿（唯一新增 = 控制失败反馈 delta）；输入延迟不劣于现状；
- 框架与 `contracts` 零改动。

**Non-Goals:**
- 服务状态页/按钮 UX（既有 deferral，归后续 GUI spec change）；
- 右槽任何 agent 入口；左槽 UI 结构调整；
- 原生 RPC 直出 GPUI 渲染（远期文档，明确不做）；
- dylib 插件化会话管理、跨进程会话；
- `AnsiBuffer`/embed 线程模型本身的重构（仅换所有权归属与消费方）。

## Decisions

### D1 进程内服务（`register_internal`），不走 dylib
会话持有 embed 线程句柄与 `pi --mode rpc` 子进程，热插拔"会话管理器"无意义；dylib 边界（指纹、永不 dlclose、TypeId fork）纯付成本无收益。框架已有 `ServiceOrigin::Internal`（`ServiceFactory = fn(ServiceCtx) -> Box<dyn Service>`，与 DLL 同契约 D1）→ **框架零改动**。
替代：dylib 插件（被否，理由如上）。

### D2 薄服务 / 渲染层切分：`AgentEntity` trait 溶解
```
crates/agent-service (新, 单 crate 两半)
├── AgentService  (impl service_core::Service)
│   ├── 会话所有权: SessionId -> SessionHandle { embed, name, root, status, size_cache }
│   ├── RPC 转发: key/text/wheel/click/resize/theme -> UiEvent (embed 注入通道)
│   ├── 状态观测 (不解析内容): 首帧字节非空 = Running; embed 循环终止 = Exited
│   ├── 50ms drain: take_ansi -> 变了才推 FrameBytes (全推)
│   ├── 投影: 任何注册表/状态/active 变化 -> SessionsChanged{Arc<SessionView>}
│   └── 未来纯逻辑落点: MCP 注入/工具注入 (config 级, 不碰屏幕)
└── AgentTerminal (impl gpui Panel; 原 TerminalPanel 改名迁入)
    ├── vt100::Parser x N (每会话一个, 连续喂养 -> 切换零成本)
    ├── build_grid/选区/scrollback/PaneData/字体度量 (全部既有代码原样迁)
    └── 出站一律: listener.submit / tell -> Router
```
- `spawn` 失败（pi 缺失）在服务侧发生 → ask 回 `Err` → GUI notice（spec delta 的 create 场景）；
- 服务侧不再有 parser：`SessionStatus` 由"字节到达/线程死亡"导出，与今天 `Starting -> Running -> Exited` 的**语义一致**（Running 定义 = 已有输出）。
替代（探索否决）：
- P1 服务解析 + `Arc<Screen>` 克隆推（每帧 96–192KB clone + 切换快照机制，被否）；
- 门铃 + `Arc<RwLock<Parser>>` 共享（数据走共享内存不走消息，破"全 message"定约，被否）；
- 只推 active + 服务 replay 历史（全量字节留底无限涨、截断重建不可靠，被否）。
**全推**（所有会话、仅 bool=true 时、idle 零消息）是用户定案：换来 pane 持 N 个连续 parser、切换零成本、零 replay。

### D3 消息契约（kind / payload / 方向）
| 方向 | kind | payload | 形态 |
|---|---|---|---|
| GUI → 服务 | `select` | `SessionId` | **ask**（用户操作一律 ask, D6） |
| GUI → 服务 | `create` | `project_root: PathBuf` | ask（回 `Result<SessionId, String>`） |
| GUI → 服务 | `close` | `SessionId` | ask |
| GUI → 服务 | `input` | `InputKey/InputText/InputWheel/InputClick` + `SessionId`(=active) | **tell**（静默失败, D6） |
| GUI → 服务 | `resize` | `(u16,u16)` + SessionId | tell（服务侧去重缓存后转 embed） |
| GUI → 服务 | `watch` | 无 payload，**`from` 即订阅地址** | tell（握手 = 存 watcher `InstanceId`） |
| 服务 → GUI | `frame_bytes` | `FrameBytes { session: SessionId, bytes: Arc<Vec<u8>> }` | tell → `Route::Instance(watcher)` |
| 服务 → GUI | `sessions_changed` | `SessionView { Arc<[SessionInfo]>; active: Option<SessionId> }` | tell（**事件自带快照**, Listener 零回程） |
| 服务 → GUI | 回复 | create/select/close 的 `Result` | ask 回执 → mailbox；失败/超时另有 `KIND_ROUTE_ERROR` |
| 框架 → GUI | `state_change`/`route_error` | 既有 | 既有（6.1） |

- 类型全部定义在 `agent-service` crate（宿主同二进制、非 dylib 边界 → 不进 `contracts`；TypeId 同侧稳定）；
- 订阅用 `from` 地址（D5 受限 `Address` 恰好是这个用途），不加框架订阅 API；
- `frame_bytes` 遇 mailbox 满（`try_send` → `ChannelFull`）**丢帧不阻塞**：帧自含全量状态，下一帧自愈；丢帧计数留服务侧观测。

### D4 active 归服务，GUI 只做镜像
spec delta 要求"选择失败 → 原 active 不变"，且切换由消息触发 → **active 是域状态**，归服务唯一权威。服务在 `close` 自动调"剩余最后一个/无则 None"（今天 `app.close_agent` 的逻辑迁移）。GUI `app.active` 降级为镜像字段（由 `SessionView` 事件驱动），中心 tab 切换仍由 GUI 在收到成功回执后执行（dock 行为本就属 GUI）。
替代：GUI 自持 active、消息只作通知（被否——选择失败反馈无法成立，切换语义双源易分叉）。

### D5 drain tick 与输入延迟预算
- 服务侧 drain 循环经 `ctx.messaging.spawn_task`（宿主 runtime，D12 既有约束）跑 50ms interval；
- **输入内联 drain**：`handle(input)` 转发完毕后立即 drain 一次（输入即突发，替代 `kick_input_drain` 的 8ms×4 在服务侧的等价物）；
- **GUI 侧快泵保留**：帧入 mailbox 无法跨线程碰 gpui，唤醒仍是 GUI 自己的事——输入是 UI 线程发起的，submit 后自踢快泵（既有 burst 逻辑改挂到"发出 input 消息"上）；服务自发帧走既有 50ms 泵相位。
延迟预算与实测（task 5.2，2026-11-19，全链路探针 = tell → embed 本地回显 → 服务 drain → 推帧 → GUI 泵消费，墙钟采样 ×5）：

- **回改记录**：初版（仅输入内联 drain + 主 tick）实测 **48–52ms**，被服务
  50ms tick 相位主导——超"既有 burst 档"（GUI 快泵 8ms×4 = 32ms 窗口）。
  按任务要求回改方案（未放宽预算）：**服务侧输入后三拍补采**
  （8/16/24ms 真实时钟自寻址 `drain.tick`，与 GUI burst 同款语义，
  ChannelFull 丢拍/实例死亡自退出），单测 `input_spawns_followup_burst_ticks` 锁行为。
- **回改后（探针）**：稳态 **31–34ms**（≤32ms burst 档 ✓，回显 ~1-2ms → 8ms
  补采捕获 → GUI 快泵窗口内消费）；冷启动首键 ~52ms（embed 首键初始化
  延迟，落主 tick 兜底，仅首轮）。
- **二次回改（手动验收数据驱动，2026-11-20，临时 AGENT_DIAG 分段计数三轮，
  量完即拆）**：真实使用中长尾 60–81ms 暴露补采窗（24ms）与 GUI 快泵窗
  （32ms）都短于"embed 空闲后单键/积压帧"的重绘晚到（~40-50ms），落
  50ms 主 tick 后双相位叠加 → 回改：**服务补采 3 拍 → 6 拍（8..48ms 无缝
  接主 tick）、GUI 快泵窗 4×8 → 10×8（80ms）、`set_view` 加 `Arc::ptr_eq`
  快路径**（泵 20+次/秒的深比较纯浪费）；单测随参数同步（6 拍/60ms 窗）。
- **终版实测（三轮分段计数）**：打字回显典型 **9–18ms**（n=254+）、渲染
  1–2ms/次、喂帧 `parser.process` ≤0.5ms、帧 0.3–4.3KB（viewport 增量）；
  `seen=fwd`/`tell_err=0`/`dropped=0`/`pushed≈fed` — 焦点丢失、静默丢键、
  消费段堵塞三项嫌疑全部排除。
- **长会话滚动归因（终判, term-rate 全链日志四轮实测）**：
  - **绘制侧（帧→画）4–13ms 三轮稳定** — gpui repaint 无滞后, 彻底排除；
  - 滚动典型 `输入→帧` **18–25ms**（25ms 主 tick 生效, 对比 50ms 时代
    52–77ms ≈2.5× 改善）+ 帧→画 4–13 ⇒ **典型滚动总链 ≈10–38ms**；
  - 偶发 342/856ms 尖峰（n 极小）: 边界滚动/无效果键的计时伪影 或
    pi 长文极慢重绘 — echo 已加 300ms 窗口隔离（`miss=` 单独计数）；
  - **结论**: 管线侧（喂帧/绘制/相位）无可挖余量, 残余是 **pi 侧重绘
    O(transcript)** —— 与"原生控制台滚动零往返"的架构性差距, 后续信号
    入 `Docs/pending-decisions.md`（方向: pi 渲染增量化/虚拟列表）。
- **三次回改（长文滚动验收, 2026-11-20）**：单次刻意滚动（滚一下停一下，
  长文浏览典型节奏）在补采窗 48ms 关闭后落 50ms 主 tick 相位——实测滚动
  52–77ms 中最多 50ms 是纯等相位 → **主 tick 50→25ms**（成本: 40 次/s
  µs 级空 drain <0.1% CPU）。复测见验收记录。
- **自发输出（无输入触发）**：双相位 = 服务 drain tick ≤25ms + GUI 泵 ≤50ms
  → 结构上 ≤75ms（初版 50+50 ≤100ms；旧为 GUI 单相位 ≤50ms）——
  task 5.2 明示的"双相位说明"即此；连续输出有帧合并红利（下一相位一并
  消费，ServiceListener 合并语义），TUI 交互场景可接受。
- 探针为临时测试（量完即删），方法与数值以本段为准。
替代：监听线程 `cx.notify` 直唤醒（gpui 跨线程 notify 需 executor hop，收益仅覆盖自发输出档，复杂度不值——记入 Open Questions 可后评）。

### D6 输入/渲染线程模型：解析仍在 UI 线程（零回归）
`take_ansi` 字节经 mailbox 到达后在 UI 线程喂 N 个 parser——与今天 `poll_sessions` 同线程同成本，只是主体从 app 换成 `AgentTerminal`。`SessionView`/parser 的可变访问全部发生在 gpui 主线程，**无需任何锁**。

### D7 crate 布局与依赖边
```
GUI (balabala-agent-gui)
 ├── service-manager (facade, 不变) ──> service-core
 ├── contracts (不变, 仍服务 dylib 边界)
 └── agent-service ──> service-core (+ gpui-kit, vt100, pi-shell-rs, crossterm)
```
- `agent-service` **不依赖** `service-manager`（工厂拿 `ServiceCtx` 已含 `messaging`/`runtime_token`；`Listener` 是 GUI 侧类型）；
- GUI bootstrap 扩展：`register_internal("agent", agent_service_factory)` + `load` + 订阅状态 + 发 `watch`；
- 控制台 tab 标签 "Console" 不变（spec 用词），仅类型/模块改名（`src/pane` → crate 内 `terminal` 模块，`TerminalPanel` → `AgentTerminal`）；
- **出站接缝 `TerminalBus`**：`agent-service` 定义窄 trait（`ask(kind, payload)` / `tell(kind, payload)`，入参只用 `service-core` 词汇），GUI 在构造面板时注入 `Listener::submit` 包装实现——终端出站不触 `service-manager` 类型，依赖边保持 `GUI -> agent-service -> service-core`。入站（帧/镜像）由 GUI 泵以普通方法调用注入（`feed` / `set_view`），面板不需要知道 mailbox。
- **tokio（task 2.6 补充）**：drain 循环需要 timer，`agent-service` 增加 tokio 依赖，features 与 `service-manager`/`service-plugin`/`example-plugin` **完全一致**（同一 workspace 编译单元，D12 教训）；本 crate 与宿主同二进制、非 dylib 边界，无 TypeId 分叉风险。

### D8 单 mailbox + 泵内路由
GUI 仍**只有一个 `Listener`**（容量 64）：`ServiceLayer::pump` 按 kind 分流——`frame_bytes` → 转交 `Entity<AgentTerminal>`（`feed(session, bytes)`，UI 线程喂 parser）；`sessions_changed` → 更新 `SessionView` 镜像 + `notify_panels`；回复/`route_error` → 失败 notice（delta 场景）。
替代：AgentTerminal 自持第二个 mailbox（多一处泵接线，且 pane 是面板不是 actor——它的泵就是 GUI 泵；被否）。
容量论证：pump 50ms 全量取件，占用 ≈ 各流速率×周期 ≈ N 个量级，远离 64；UI 停摆时满 → 服务丢帧自愈（D3）。

### D9 测试归属与行为保持
- 纯逻辑单测（注册表/状态导出/去重/active 退位）落 `agent-service` crate；
- 需要总线的集成测（控制 ask→回执→notice、watch→frame 推送、select 失败反馈）落 GUI crate（dev 依赖图同时有两边）——`agent-service` 不加对 `service-manager` 的 dev-dep，守住 D11 边（GUI 本就二者兼备）；
- pane 既有测试随代码迁 crate，断言不改；app 级测试（门控/主题/轮询）改注入点、不改语义；
- **基线：`cargo test --workspace` 234 测全绿 + 既有 spec 场景全过**（含 delta 的三个新场景）。

## Risks / Trade-offs

- **~~`EmbeddedApp: Send` 未验证~~ → spike 已过（task 1.1, 2026-11-19）**：`EmbeddedApp` 与 `PiAgentShell` 均 `Send + Sync` 编译期断言通过（字段 = Sender/Arc<Mutex|RwLock|AtomicBool>/JoinHandle，无裸指针）——所有权可直接进 kameo driver，**专用线程退路不需要**。假设前提不变时无需重评。
- 输入链路多两次 channel 往返（UI→Router→服务→embed→字节→mailbox→UI）→ 内联 drain + 快泵兜底（D5 预算）；集成测压一道"输入后一帧必达"的时序断言。
- mailbox 满丢帧 → 帧自含全量状态、后续帧自愈；丢帧计数上观测面，阈值告警留观。
- 大迁移回归面（164 个 GUI 测试触及 session/pane 内部）→ 分阶段任务内保持"先迁代码再改断言"，每阶段全量绿再进下一阶段。
- `close`/`select` 从同步变异变为 ask 回执（ms 级异步）→ 回执前重复点击/双请求由服务侧幂等处理（未知 id → `Err`，不重复关）。
- 帧 `kind` 全局共用、快照 map 按 kind 覆盖 → 渲染不走 `snapshot()` 而走本地 parser（D6），`sessions_changed` 单 kind 最后值语义恰好正确；不踩混流坑。

## Migration Plan

阶段顺序即任务组顺序，**每阶段全量测试绿再进下一阶段**，单阶段可独立回滚（阶段 1-2 纯新增；阶段 3-4 是移动+改写，git mv 保历史）：

1. **Spike + 骨架**：`EmbeddedApp: Send` 断言与冒烟；`crates/agent-service` 建仓，`AgentService` 最小 `start/handle`（空注册表）注册进 GUI bootstrap（load 成功、状态事件到 pump）。
2. **服务全功能（无 UI）**：注册表/status 观测/control 五 handler/watch 握手/frame 全推/投影事件/drain 接线——以 `TestMessaging`（`service-core` 既有测试模式）单测驱动，测试假会话（可注入字节的 stub）代替真 pi。GUI 尚不动。
3. **GUI 数据/消息面换底（pane 暂留 `src/pane`）**：Entity 消亡（SessionId/workspace/sidebar）、active 归服务、ServiceLayer 扩展（watch 握手/泵分流/失败 notice=delta 场景）、输入 tell+快泵重挂、pane 渲染源切到本地 parser + `feed`、主题/resize 改 tell、`src/agent_entities` 溶解。**先换逻辑、组末全量绿基线**——此阶段不搬文件，断言改注入点不改语义。
4. **AgentTerminal 迁 crate 改名**：逻辑稳定后再 `git mv src/pane` 进 crate、`TerminalPanel` → `AgentTerminal`、`TerminalBus` trait 落位（GUI 注入实现）、测试按归属分家（纯渲染随迁、泵/门控留 GUI）。纯结构移动，保 git 历史。
5. **收口**：全量 234+ 测绿、输入延迟实测对照写回 D5、`Docs/pending-decisions.md`（IO 条目随所有权迁移更新）与 design 事实写回。

## Open Questions

（可安全后答、不改 specs/方案/任务拆分者；其余均已上文定案。）

- 非活跃会话帧是否降频/合并（全推现状无压力，真实多开会话数上来再评）；
- 服务自发输出的 UI 直唤醒（executor hop 方案）——只有当 50ms 相位被实测抱怨时再开；
- `SessionView` 投影是否加节流（事件自带快照 + Listener 合并红利，当前无观测到的抖动）。
