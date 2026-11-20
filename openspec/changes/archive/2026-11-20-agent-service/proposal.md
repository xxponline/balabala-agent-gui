# Proposal

## Why

`service-management-framework` 立好了总线骨架，但唯一常驻消费者是假服务（`core.fake`）——框架的抽象还没被真实领域形状顶撞过。GUI 的核心领域（agent 会话）至今是直调架构：`Entity<AgentSession>` 由 workspace 持有、终端面板直接伸进 `TerminalApp` 拿数据、渲染与会话生命周期揉在同一批类型里。这个 change 落第一个真实领域服务，把"薄服务 + message 通信 + 渲染层独立"的切分确立为后续模块（AssetManager 等）迁移的模板。

## What Changes

- **新增 workspace crate `agent-service`**，与 GUI 宿主同进程，内含两半：
  - **`AgentService`（薄服务）**：实现框架 `Service` 契约经 `register_internal` 注册（**框架零改动**）。持有全部会话的 `EmbeddedApp` 所有权，负责：控制面 RPC 转发（key/text/wheel/click/resize/theme → `UiEvent`）、状态观测（首帧到达/线程死亡，不解析内容）、`(cols,rows)` 去重缓存、50ms ANSI drain 与帧外推（输入后另有 8/16/24ms 三拍补采压回 burst 档），未来 MCP/工具注入等纯逻辑也落这里。
  - **`AgentTerminal`（渲染层，由 `src/pane` 的 `TerminalPanel` 改名迁入）**：独占 vt100 `Parser` × N（每会话一个）、`build_grid`/选区/scrollback、`PaneData`。对服务只发消息、只收消息。
- **`AgentEntity` trait 溶解**：控制面归服务侧，渲染面（parser+grid）归 AgentTerminal；`AgentSession(gpui Entity)` 包装消亡——会话以服务自有 `SessionId` 注册表归服务持有，GUI/sidebar 经服务事件投影（`SessionView { Arc<[SessionInfo]>, active }` 快照）读列表与状态。
- **全部控制走消息**：左槽 sidebar 的 `SelectAgent/NewAgent/CloseAgent`、输入注入、resize/主题热推 → Router（用户操作一律 ask，D6；输入注入 tell）。帧契约：服务对**所有**会话推 `FrameBytes{session, Arc<Vec<u8>>}`（字节所有权移动、零拷贝；仅 `poll` 有新字节时推、idle 零消息；订阅 = GUI 以自己 `from` 地址握手）。GUI 泵的输入快路径（现 `kick_input_drain`）在新链路上重做。
- **UI 行为保持**：左槽功能与 spec 原样（只换控制底），右槽不涉 agent 职责，中央仍渲染 console。唯一**新增可观察行为**：消息化控制的失败面（见 capability delta）。
- **文档**：远期方向已记入 `Docs/future-agent-terminal-native-rpc.md`（原生 RPC 直出 GPUI，近期不做）；`Docs/pending-decisions.md` 中受影响条目随 design 写回。
- **BREAKING（仓内结构，无外部消费者）**：`src/pane`、`src/agent_entities` 迁出/溶解为 `crates/agent-service`；`TerminalPanel` → `AgentTerminal`；`Workspace::agents` 从 `Vec<Entity<AgentSession>>` 改为 `Vec<SessionId>`。

## Capabilities

### New Capabilities

（无——不引入新能力；架构换底不产生新能力面。）

### Modified Capabilities

- `agent-shell`：新增"控制消息失败反馈"需求——会话控制（切换/新建/关闭）改经服务消息后，ask 失败/超时成为新的可观察面：操作不生效时状态保持原样且 UI 出 notice（复用现有 notice 机制）。其余既有需求（生命周期、渲染、焦点模型、输入、字体契约等）行为全部保持，无修改。

## Impact

- **代码**：`src/app.rs`（active/建停/主题/泵接线）、`src/workspace`（agents 容器）、`src/sidebar`（行载荷改 `SessionId`）、`src/pane` → `crates/agent-service`、`src/agent_entities` 溶解、`src/service_layer.rs`（注册/订阅/握手扩展）。
- **依赖边**：新增 `GUI → agent-service → service-core`（+ gpui-kit/vt100/pi-shell-rs/crossterm 由 agent-service 承载）；GUI → service-manager 不变；**service-manager/service-core/service-plugin/contracts 零改动**。
- **测试**：全工作区 234 测为基线——pane 相关测试随迁 agent-service 并保持断言；`send_key`/轮询/门控等 app 级测试改注入口；新增服务控制、帧推送、订阅握手、失败反馈测试。
- **风险面**：`EmbeddedApp: Send` 已 spike 证实（编译期断言，退路不需要）；输入延迟已实测并回改一轮（design D5：初版 48–52ms 超档 → 服务侧三拍补采 → 稳态 31–34ms ≤ burst 档；自发输出 50+50ms 双相位说明在案）。
