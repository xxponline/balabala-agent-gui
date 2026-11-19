# 远期计划：AgentTerminal 直接消费 pi 原生 RPC，GPUI 层原生渲染

> 状态：**远期方向记录 — 近期不做**（用户决策 2026-11-19）
> 关联：`openspec/changes/archive/2026-11-19-service-management-framework`（服务骨架）、
> 后续 agent-service change（待开，薄服务 + 渲染层归 AgentTerminal + 字节全推）、
> `openspec/specs/{agent-shell}`（现行终端渲染行为约束）
> 性质：渲染层换代的远期规划。**不改变近期任何计划**，仅记录方向、钩子与触发条件。
>
> **变更记录**
> - 2026-11-19：初版。源于 agent-service 探索中"帧数据零拷贝"的追问——既然近端
>   已决定薄服务只推 ANSI 字节、渲染层整体归 AgentTerminal，那"不经过终端编码、
>   直接从 pi 原生 RPC 转发到 AgentTerminal 在 GPUI 层构建界面"作为远期换代方向
>   记录在案。

---

## 1. 今天的数据流（两段 ASCII 化）

```
pi --mode rpc 子进程 (JSONL stdout)
  -> pi-shell-rs session 层把事件拼成 TUI (transcript/input/tool 卡)
  -> VirtualBackend 画成 ANSI 字节                 <-- ASCII 化 ①
  -> take_ansi() 拿到所有权
  -> AgentService 50ms drain -> message 全推 (Arc<Vec<u8>>, 移动零拷贝)
  -> AgentTerminal 的 vt100::Parser 解析成 cell 网格  <-- ASCII 化 ②
  -> build_grid -> gpui Div (cell 网格整套契约:
     字体度量/无漂移/选区坐标/宽度下限/宽字符占位...)
```

**①② 两段都是"假装是终端"付的税**：结构化的 agent 事件流被压成 ASCII，再被
逐 cell 解回来。cell 网格 UI 的全部复杂度——monospace 字体挑选与度量、
网格无横向漂移契约、坐标映射选区、宽字符占位——都是这次往返的衍生成本
（见 `openspec/specs/agent-shell` 中相关 requirement，它们是现行行为约束，
不是这个方向要删的 bug）。

## 2. 远期目标

```
pi --mode rpc (JSONL)
  -> ClientEvent::Event(Value) / extension_ui_request
  -> [typed 提取层: 待设计]
  -> AgentTerminal 用 gpui 原生组件直接构建界面
     (真实文本块/按钮/卡片/滚动视图, 无 vt100, 无 cell 网格)
```

- ①（VirtualBackend → ANSI）与 ②（vt100 → cell）**整体消失**；
- 输入侧从"合成 crossterm KeyEvent 打进终端"变成"结构化命令面"；
- 现有 cell 网格渲染层（`build_grid`、网格度量、选区契约）成为**可整棵弃换的叶子**
  ——这正是近端切分"渲染层独立归 AgentTerminal"买到的远期红利：服务层与消息
  通道不动，只换叶子。

## 3. 已知钩子（pi-shell-rs 侧实测，2026-11-19）

| 钩子 | 位置 | 形态 |
|---|---|---|
| `ClientEvent::Event(Value)` | `rpc/client.rs` | id-less 事件流：session events / `extension_ui_request` / `extension_error` |
| `RpcClient::call()` + `OutgoingCommand` | `rpc/client.rs` / `rpc/jsonl.rs` | 带 id 的命令面（请求/应答/deadline 齐全） |
| `WireMessage::top_type()` | `rpc/jsonl.rs` | 顶层 `type` 判别字段（路由/分类用） |
| `UiEvent` 注入 | `app/event.rs` | 宿主 → embed 反向通道（Key/Text/Resize/Shutdown），近期输入仍走它 |

`extension_ui_request` 是 pi 主动请求宿主 UI 的现成钩子——远期最可能的
渐进入口（先接管一部分 UI 请求，而不是一次性重写整个 transcript）。

## 4. 与近端切分的关系（近切分不白做）

- 近端薄服务的 RPC 转发面（`send_key/... -> UiEvent`）与远期是**同一条路**：
  届时只是 message payload 从 `FrameBytes(ANSI)` 换成 typed RPC 事件，
  `AgentTerminal <-> AgentService` 的消息通道、订阅/推送机制原样复用；
- 服务侧保持"不碰屏幕内容"的立场不变——MCP/工具注入等纯逻辑继续落薄层，
  原生事件流的消费方自始至终是 AgentTerminal；
- vt100 parser 只存在于叶子内，弃换时不需要动 `ServiceManager`/Router/Listener
  任何一层。

## 5. 触发条件（什么信号值得启动）

1. **cell 网格天花板被顶穿**：字体度量/选区/组件交互（下拉、按钮、卡片）
   在"假装是终端"的路子上修无可修，继续投入的边际成本超过换代；
2. **pi 事件面稳定下来**：见 §6——payload 还在演化时固化 typed 契约会反复返工；
3. **pi 侧主动推力**：`extension_ui_request` 类钩子覆盖的场景变多，
   "宿主原生渲染"成为 pi 生态的推荐姿势。

三条不齐，不动。**近期（本次 change 及可见的下几个 change）明确不做。**

## 6. 风险与未知

- **TUI 组合逻辑在 pi-shell-rs session 层**（transcript 拼装、输入框、tool 卡
  都是它画的）：原生化 = 用 gpui 重写这层组合，不是薄薄换一个渲染后端——
  工作量主体在这里，不在 AgentTerminal；
- **`extension_ui_request` 能覆盖多少未知**：若能覆盖大部分非 transcript UI，
  渐进接管可行；若只能覆盖边角，则是全量重写 vs 长期双轨的取舍；
- **payload 演化中**：pi-shell-rs 对 wire 的立场是 "only the discriminator
  fields needed for routing are extracted; raw JSON is preserved"——typed
  提取层必须后于 pi 事件面稳定，过早固化 = 追着 pi 改；
- **vt100 是否留作 fallback**：raw 输出场景（日志行、非结构化 stdout）是否
  需要终端兜底、能否与原生块混排，未调研；
- **双轨期的切换成本**：渐进接管期间 cell 网格与原生组件共存，布局/滚动/
  选区跨两套渲染——若走渐进路线，这个过渡形态要先设计。
