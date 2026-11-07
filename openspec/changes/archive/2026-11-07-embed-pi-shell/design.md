# Design

> 研究归档：本文固化 2026-07 explore 阶段的架构推演。动机见 proposal.md；
> 需求条目变更见 specs/（后续补齐）。

## Context

当前两条技术栈并存：

```
balabala-agent-gui (GPUI)                     pi-shell-rs (path 依赖 ref/pi-shell-rs)
================================              =======================================
sidebar: workspace / pi-N                     lib: rpc (spawn `pi --mode rpc`)
AgentEntity trait (src/agent_entities/)             + session (SessionState/Block/Entry)
  PiAgentPty:                                    bin (mod app 私有于 main.rs):
    portable-pty 起 `pi` 原生 TUI                - spawn_event_sources: 一条 ui_rx 通道,
    字节流 -> vt100::Parser                      挂两个生产者 (crossterm poll 线程,
    send_input(&[u8]) 回灌键盘                    RPC reader 线程)
main pane: vt100 cell 渲染 (ui/pane.rs)         - event_loop: recv_timeout(200ms) -> handle
                                                   -> draw(&mut DefaultTerminal) [Stdout]
                                                 - 3 处摸真实终端: TerminalGuard(raw/alt),
                                                   SetTitle, OSC52 写 stdout
```

探索中验证过的关键事实（缝隙证据）：

1. **输入已经是通道化的** — `app/event.rs` 的 `spawn_event_sources` 给一条
   `ui_rx` 挂多生产者，`UiEvent::Key(crossterm::KeyEvent)` 只是其中一种消息；
   新增一个生产者（GUI 注入）不需要动 event_loop。
2. **渲染已经有 view-model 缝** — `view.rs:43`
   `render(frame, state: &SessionState, ui: &ViewUi)` 是纯投影函数，输入只有
   两个结构；测试用 `TestBackend` 做快照断言，证明 draw 不依赖真实终端。
3. **GUI 侧也有缝** — `AgentEntity` trait 把"会话"抽象好了，
   `screen() -> &vt100::Screen` 是渲染契约。

## Goals / Non-Goals

**Goals:**

- 进程内嵌入 pi-shell-rs，消灭 agent 会话路径上的 PTY；唯一子进程剩
  `pi --mode rpc`（stdio 管道）。
- 渲染保持 cell 路线：ratatui → ANSI → vt100 → 现有 pane.rs，**pane.rs 零改动**。
- GUI 能读到结构化 `SessionState`（为 sidebar 状态升级解锁能力，不在此实现）。
- pi-shell-rs 的独立 TUI bin 继续可跑，作为开发/回归 harness（probe、
  TestBackend 快照测试复用）。
- 为未来第二渲染方案（GPUI 直绘）保留零成本的缝，但现在不构建。

**Non-Goals:**

- 不构建 GPUI 原生渲染器、不加 Renderer trait / feature flag / 配置开关。
- 不做 sidebar 结构化状态（token/工具/模型）的 UI。
- 不做 TUI 内鼠标支持（pi-shell-rs 标记为 stage 2）。
- 不动 `workspace-management` 能力。

## Decisions

### D1: 进程内嵌入，否决"PTY 里跑 pi-shell-rs 二进制"

备选是把 `PiAgentPty` 起的 `pi` 换成 `pi-shell-rs` 二进制——两侧零改动、
渲染路径也不变。否决理由：那是个黑盒，GUI 永远拿不到内部状态；而 pi-shell-rs
的存在意义就是被嵌入并接受大量针对性修改，进程边界会把每次修改都变成 IPC 协议
修改。嵌入后进程数 3 → 2（GUI、pi-rpc），生命周期从 kill 子进程变成 drop +
优雅关闭 RPC。

### D2: 渲染走 α（ANSI → vt100），β（ratatui cells 直出）留作优化

- α：VirtualBackend 把 crossterm 后端的增量 ANSI 写进内存缓冲，喂现有
  `vt100::Parser`。pane.rs、cell 合并、光标块、颜色映射全部不动；
  `AgentEntity::screen()` 类型不变。
- 代价：每帧一次 ANSI 编解码往返。真终端本来就收这些字节（crossterm 后端
  就是 diff 增量写），保真度等价于"一个真实终端收到的流"。
- β 未来可切换点：`draw` 泛型化后换 Backend 即可，不需要回头改 α 路径。
- 结论：vt100 层定位为 ratatui 投影的**传输带**，不属于 pi-shell-rs 渲染本身
  （这正是"剥离"的含义）。

### D3: 输入 = ui_rx 通道的第三个生产者

```
producer A: crossterm poll 线程 (现有, 嵌入模式下不 spawn)
producer B: RPC reader 线程     (现有, 不动)
producer C: GPUI 键盘/IME  (新) -- 合成 UiEvent::Key(KeyEvent) --> ui_rx
```

- 改造点：`spawn_event_sources` 多返回一个 key sender（或拆出可外部注入的
  入口），event_loop 一行不改。
- GPUI KeyEvent → crossterm KeyEvent 合成；IME 已提交字符串 → `Char` 事件
  逐字合成，与现有规格"只转发已提交字符串、不转发 preedit"语义一致。
- 依据：pi-shell-rs 已有 `input_tests` 对 `handle_key` → intent 路由做断言，
  合成层的键位语义有现成回归网。

### D4: 三处真实终端 touchpoint 换成 sink

| 现状 | 嵌入模式 |
|---|---|
| `TerminalGuard::enter()` raw/alt-screen | 跳过（无 tty 可设置） |
| `SetTitle` 写 stdout | 回调 → GUI 真窗口标题（升级） |
| OSC52 剪贴板序列写 stdout | 回调 → GPUI 真剪贴板（升级，PTY 路线里是死输出） |

### D5: `draw(&mut DefaultTerminal)` 泛型化

签名改 `impl ratatui::backend::Backend`（或泛型参数）。已有
`TestBackend` 测试（`app/mod.rs:5379`）证明可行；调用点单一（event_loop）。
新写的 `VirtualBackend` 实现同一 trait：维护 ANSI 缓冲/或 cell 缓冲 + 终端
尺寸，GUI 侧按需取走喂 vt100。

### D6: `AgentEntity` trait 演化

```
保留:   screen() -> &vt100::Screen   (α 路线类型不变)
        poll() / status() / shutdown() / name() / ws_path()
演化:   send_input(&[u8])  -->  send_key(KeyEvent) + send_text(&str)
        理由: GPUI 产出的本来就是结构化事件, 编字节再解码是绕路
```

GUI 侧新增 `PiAgentShell` 实现替换 `PiAgentPty`；trait 是内部契约，
改动半径 = 1 trait + 1 实现 + spawn 站点，pane.rs 不动。

### D7: 双渲染保险 = 状态边界纪律（唯一现在要守的）

未来 GPUI 直绘渲染器 = `view::render` 旁边的兄弟函数，吃**同样两个输入**
(`SessionState`, `ViewUi`)。现在唯一要做的：

> 新功能的状态只能进 `SessionState` 或 `ViewUi`；
> `view::render` 不许从 `App` 偷读隐藏状态。

违反一次，未来分叉就多一个漂移点。不提前建 Renderer 抽象——为不存在的
第二个实现付租金没有意义。

### D8: 两仓分工的墙

```
pane 内容 (对话怎么呈现/交互)  --> pi-shell-rs, 针对性修改主要落这里
shell 外壳 (sidebar/workspace/  --> balabala-agent-gui
            窗口/会话生命周期)
embed API (窄)                 --> 两者之间唯一的墙
```

pi-shell-rs 侧改造清单（四步，全部落在已验证的缝隙上）：
1. `mod app` 从 bin 私有提进 lib（`main.rs:3`）；
2. `spawn_event_sources` 暴露 key sender，嵌入模式不 spawn crossterm 线程；
3. D4 三处 sink 化；
4. D5 draw 泛型化 + VirtualBackend。

## Risks / Trade-offs

- **[ANSI 往返出现序列不兼容]** → crossterm 输出的是标准 CSI/SGR，vt100 即
  真实终端语义；若个别序列丢失，切 β 是单点替换（D2），不需要回头改架构。
- **[线程模型: app 线程 vs GPUI 渲染线程]** → 复用现有 50ms poll 节流 +
  `screen()` 快照读取模式；VirtualBackend 的缓冲用互斥/双缓冲传递，帧同步
  协议在实现期定（见 Open Questions 之外的实现细节）。
- **[外部编辑器 Ctrl+G 无 tty]** → 嵌入模式下 `$EDITOR` 子进程没有可继承的
  终端；实现期改走 GUI 文本编辑路径（rfd 或 GUI 编辑浮层），见 Open Questions。
- **[path 依赖的本地漂移]** → pi-shell-rs 就是本项目组自己的仓库，path 依赖
  是有意选择（见 Cargo.toml 注释）；发布时再切 git rev（已记录于会话结论）。
- **[键位保真度]** → 规格要求的键集（可打印字符、Enter/Backspace/Tab、
  方向键、IME 提交）全部经合成层落到 crossterm KeyEvent；pi-shell-rs 的
  input_tests 是回归网。

## Migration Plan

1. **pi-shell-rs 侧**（独立仓库，不阻塞本仓库）：四步改造，bin 全程保持
   可跑（harness 绿 = 回归通过）；新增 embed 模式的快照测试
   （VirtualBackend → vt100 → 断言文本，与 TestBackend 测试并列）。
2. **GUI 侧**：新增 `PiAgentShell`（实现 AgentEntity），spawn 站点切换；
   `PiAgentPty` 暂留一个版本周期作对照。
3. **规格**：`agent-shell` delta（机制性措辞迁移，用户可见行为保持）。
4. **收尾**：移除 `portable-pty` 依赖与 `PiAgentPty`。

回滚策略：`AgentEntity` 是单点替换，切换失败只需换回 spawn 站点的实现类型；
两仓改动互不阻塞（pi-shell-rs 改造期间 GUI 维持现状）。

## Open Questions

- **双渲染的选择粒度**：未来 GPUI 直绘是 per-session / per-workspace /
  全局切换？用户明确暂缓，等第二渲染器真正启动时再定。
- **嵌入后 TUI 内鼠标**（点击卡片/滚动）：pi-shell-rs 现为 stage 2 未实现；
  通道模型已预留（加 `UiEvent::Mouse` + GPUI 坐标换算），需要时再开。
- **外部编辑器（Ctrl+G）在无 tty 环境的形态**：实现期决定（GUI 编辑浮层 vs
  其他），不改变本设计的架构与任务边界。
- **sidebar 结构化状态展示什么**：能力已解锁（SessionState 只读句柄），
  展示内容与样式属于后续需求，不在本 change。
