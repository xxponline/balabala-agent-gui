# Tasks

## 1. Spike + 骨架

- [x] 1.1 `EmbeddedApp: Send` spike：在 `agent-service` 加编译期断言测试（`fn assert_send<T: Send>()` 覆盖 `PiAgentShell`/`EmbeddedApp` 及服务持有的句柄），验证所有权可进 kameo driver；**验证**：`cargo test -p agent-service` 断言测试通过，spike 结论（或不满足时的退路线程模型）写回 design.md Risks 条目
- [x] 1.2 建 `crates/agent-service` 骨架：Cargo（依赖 service-core + gpui-kit/vt100/pi-shell-rs/crossterm，**不含** service-manager）、`AgentService` 空注册表实现 `start`/`handle`、`agent_service_factory` 导出；**验证**：`cargo build -p agent-service` 通过 + 单测 `start` 返回 Ok
- [x] 1.3 GUI bootstrap 注册并装载 `"agent"` 服务（`register_internal` + `load`），状态事件进既有 pump；**验证**：新测试 `agent_service_loads_and_reports_state`（bootstrap 后 `last_state == Available`、`events_seen` 增长）在 `cargo test --workspace` 绿

## 2. AgentService 全功能（无 UI）

- [x] 2.1 会话注册表：`SessionId` 分配、服务侧 `SessionIo` 抽象（drain 字节/注入/status/shutdown）+ 测试 stub、状态观测三态导出（首帧字节非空 = Running，io 终止 = Exited，起始 = Starting）；**验证**：单测 `status_transitions_from_bytes_and_io_end` 三态迁移通过
- [x] 2.2 控制 handler：`select`/`create`/`close` 的 ask 回执（含 active 归服务、关闭 active 后退位剩余最后一个/无则 None、未知 id 与重复关的幂等 `Err`、create 失败回 `Err(String)`）；**验证**：`TestMessaging` 回执捕获单测覆盖：成功三支 + 退位 + 未知 id `Err`
- [x] 2.3 输入/尺寸/主题 handler：`input` 四种 tell 转发到 `SessionIo`、`resize` 服务侧 `(cols,rows)` 去重后再转、`theme` 广播转发；**验证**：单测 stub 捕获转发载荷 + 重复 resize 不二次转发的断言通过
- [x] 2.4 watch 握手 + 帧全推：`watch` 从 `from` 地址存 watcher、drain 变字节才推 `FrameBytes{session, Arc<Vec<u8>>}`（所有权移动）、全会话推、推失败丢帧并计数；**验证**：单测覆盖 watch 前零帧 / 变字节后到达 route=watcher / 无字节不推 / 出口满时丢帧计数递增
- [x] 2.5 投影事件：任何注册表/状态/active 变化推 `SessionsChanged{SessionView{Arc<[SessionInfo]>, active}}`（事件自带快照）；**验证**：单测断言 create/close/select/status 后事件负载为全量快照且 active 正确
- [x] 2.6 drain 接线 + 契约文档：`input` handler 内联 drain（输入即推帧）、drain 循环经 `messaging.spawn_task`（宿主 runtime 50ms interval）；D3 契约表落 crate 级 rustdoc；**验证**：内联路径单测（发 input 后 stub 字节立即成帧）通过 + `cargo doc -p agent-service --no-deps` 零警告

## 3. GUI 数据/消息面换底（pane 暂留 `src/pane`；组内按序执行，以 3.6 为全绿基线）

- [x] 3.1 ServiceLayer 扩展：发 `watch` 握手、`pump` 按 kind 分流（`frame_bytes` → pane.feed 入口、`sessions_changed` → `SessionView` 镜像、回复/`route_error` → `app.notice` 失败文案）；**验证**：新测试 `watch_receives_frame_bytes` 与 `failed_control_reply_shows_notice`（delta 场景 1：失败回执 → 状态不变 + notice 点名操作）绿
- [x] 3.2 Entity 消亡 + active 归服务：`Workspace::agents: Vec<Entity<AgentSession>>` → `Vec<SessionId>`、`app.active: Option<SessionId>` 改镜像驱动、sidebar 行载荷/列表渲染改读 `SessionView`、select/create/close 改 `submit` ask + 回执处理（成功切 console tab，失败留 notice）；**验证**：delta 三个场景（失败关/失败选/失败建）全测绿 + sidebar 既有断言（列表/计数/徽章/焦点模型）改注入点后语义不改地绿
- [x] 3.3 输入链换底：`send_key`/`send_text`/轮滚/点击 → `tell(input)` + submit 后自踢快泵（`kick_input_drain` 语义重挂）；**验证**：既有 Tab 转发与输入注入测试绿 + 新增 envelope 断言（kind/payload/active id）+ "输入后一帧必达泵"时序断言通过
- [x] 3.4 pane 渲染源切换：本地 parser ×N 落 `src/pane`（pump 把 `frame_bytes` 喂 `feed`），`screen()` 对 `AgentSession` 的依赖断除，active/状态栏改读 `SessionView` 镜像；**验证**：pane 渲染测试改 feed 驱动后绿（cell/选区/字体断言不改），`rg "active_session|AgentSession" src/pane` 零命中
- [x] 3.5 主题/resize 改 tell + `src/agent_entities` 溶解：`push_tui_theme`/网格 resize → tell 经服务转发（服务侧去重），删除 `AgentSession`/`AgentEntity`/`SessionStatus`（状态移入 agent-service 的 `SessionInfo`）；**验证**：ThemeRecorder 与 resize/门控系列测试绿，`rg "AgentSession|agent_entities" src/` 零命中
- [x] 3.6 阶段基线：全量 `cargo test --workspace` 绿、0 warnings；**验证**：命令输出对账（≥234 测，与基线计数无缺失）

## 4. AgentTerminal 迁 crate 改名

- [x] 4.1 `git mv src/pane` → `crates/agent-service/src/terminal`，`TerminalPanel` → `AgentTerminal`，`TerminalBus` trait（`ask`/`tell`，只用 service-core 词汇）落 agent-service、GUI 以 `Listener::submit` 包装注入；**验证**：`cargo build` 全仓通过 + `cargo test -p agent-service` 迁入测试全绿（纯渲染断言随迁）
- [x] 4.2 GUI 构造与 console tab 接线改 import（`property_detail_manager`/`shell`/`app`），残留引用清零；**验证**：`rg "crate::pane|TerminalPanel" src/` 零命中 + `cargo build` 通过 + console 面板测试（焦点/渲染占位）绿
- [x] 4.3 测试归属分家核查：纯渲染/度量测试在 `agent-service`，泵接线/门控/失败 notice 留 GUI；**验证**：`cargo test --workspace` 全绿且测试总数与阶段 3.6 基线对账（只增不减、无 `#[ignore]` 偷渡）

## 5. 收口

- [x] 5.1 终验：`cargo test --workspace` 全绿 0 warnings + `openspec validate agent-service` 通过；**验证**：两条命令输出
- [x] 5.2 输入延迟实测对照（输入 → 内联 drain → 快泵帧达 ≤ 既有 burst 档；自发输出 = 50ms 双相位说明）；**验证**：实测数值写回 design.md D5，与预算不符则回改方案而非放宽
- [x] 5.3 文档写回：`Docs/pending-decisions.md` IO 调度条目（会话所有权与 drain 归属已迁移）、design.md Risks/D5 结论（Send spike 结论、延迟实测）、proposal 与最终实现对账；**验证**：逐文件 diff 审阅，文档日期更新为实施当日
