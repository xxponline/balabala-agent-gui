# Pending Decisions

记录当前 demo 阶段先简化、但未来需要重新评估的设计权衡。

最近一次对齐: 2026-11-19 (change `agent-service` 落地后), 对照
`openspec/specs/` 七份主 spec (原五份 + service-lifecycle / service-routing)。
上一次: 2026-11-15 (embed-pi-shell 换代)。

## 多 agent 的 IO 调度

**当前选择 (2026-11-19 更新, 随 `agent-service` 换底)**: embed 线程模型不变
(每 session 一条 embed 事件循环线程 + RPC 泵/读写线程, 约 5 线程/session;
`AnsiBuffer` 共享无界注入不变), 但**所有权与帧 drain 归 AgentService**:
- 会话本体 (`EmbeddedApp`) 由服务持有; 服务 50ms 自 tick drain (take_ansi
  所有权取走 → `FrameBytes` 全推 GUI mailbox), 输入后另有 8/16/24ms 三拍
  补采 6 拍 (8..48ms) + GUI 快泵窗 80ms (手动验收长尾回改, 见 change
  design D5);终版实测: 打字回显 9-18ms, 喂帧 process ≤0.5ms;
- GUI `start_update_loop` 不再 drain 会话, 只消费 ServiceLayer mailbox
  (kick_input_drain 语义 = 消费侧快泵); 自发输出为 50ms+50ms 双相位
  (结构上 ≤100ms, 连续输出有帧合并红利);
- 输入 (键/滚轮/点击/文本) 改经 Router tell → 服务转发 embed `UiEvent`
  (embed 侧通道与注入语义不变); vt100 渲染 parser 迁 AgentTerminal 本地
  (每会话一个, 全推喂养, 切换零成本)。

**上一版 (2026-11-15, embed-pi-shell)**: PTY 路径已删除; 宿主 50ms 轮询
统一 drain + 输入后 8ms×4 burst。

**为什么这样选**: 与 embed 架构一致, 实现最简; 50ms 轮询对 TUI 帧粒度够用,
burst 消除了输入后的相位抖动。

**未来需要重新评估的信号**:
- session 数到 30+ 时 ~5 线程/session (≈150 线程) 是否成为问题
- 狂喷输出 (例如粘贴巨量文本) 时 `AnsiBuffer` 在两个 drain 之间无界堆积,
  vt100 单次解析洪峰
- 空闲帧显示延迟上限: 双相位 ≤75ms (服务 25ms + GUI 泵 50ms;
  change agent-service 三次回改后), 输入路径 9-18ms (打字)
- **长文滚动 vs 原生控制台的差距 (2026-11-20 终判)**: term-rate 全链
  实测 — 绘制侧帧→画 4-13ms、典型滚动输入→帧 18-25ms (25ms 主 tick
  后, 曾 52-77ms), 典型总链 ≈10-38ms; 偶发 342-856ms 尖峰 (边界无重绘
  伪影或 pi 极慢重绘, echo 300ms 窗口已隔离计数 miss)。管线无可挖余量,
  残余 = pi 侧重绘 O(transcript) — 原生控制台是本地缓冲平移 (零往返),
  TUI 架构性差距; 若需逼近, 方向在 pi-shell-rs/pi 渲染增量化/虚拟列表
  > **前提修正 (2026-11-21, change 2026-11-20-streaming-input-echo-latency)**:
  > 上述"残余 = pi 侧重绘 O(transcript)"成立于渲染链 52-113ms 时代、且未计
  > H5。流式期输入延迟 (38-617ms) 主因实为 **H5 pi-shell-rs `event_loop`
  > batch-draw 合并** (输入回显绑进大流式帧、等整批 draw), 非单纯重绘
  > O(transcript)。已修: 输入类事件优先独立 draw (详见该 change design.md
  > 归因段)。term-rate 口径: 回显帧 = 代际 > 输入基线的首帧, 但同代际不区分
  > 流式/回显 (探针盲区), H5 修复后回显才独立成小帧。
- mio / epoll 选项已作废 — 不再有 pty master fd

**备选方案**:
- A: embed 帧就绪时事件驱动通知宿主 (condvar / 回调 → `cx.notify`), 替代
  50ms 轮询 — 需要跨线程安全地打进 GPUI 调度
- B: `AnsiBuffer` 改有界 + 背压 / 帧合并 (丢中间帧只留最新)
- C: 线程池化 — 多 session 共享 I/O 线程, 降低每 session 线程数

## 终端尺寸 (resize) 同步

**状态: 已解决** (2026-11-08, change `add-adaptive-terminal-grid`)。

- spec 新增两条 requirement: **Adaptive terminal grid sizing** (网格行列由
  可用像素面积 + monospace 字体 metrics 下取整推导, 钳制到安全最小值
  20×5 — `MIN_GRID_COLS` / `MIN_GRID_ROWS`) 与 **Grid resize propagation**
  (尺寸变化同一步同时传播到 embed TUI 与 vt100 屏; 未变化不传播;
  `exited` 会话不 resize 以保留最后画面)。
- 原"要做的事"全部落地: 每帧从 grid 区域 bounds 算
  `cols=floor(w/cell_w)`、`rows=floor(h/line_h)` (text system metrics,
  与 cell-width 渲染共用同一真值); 变化时 `EmbeddedApp::resize()` +
  `vt100::Parser::set_size()` 同步调用, 同尺寸去重。
- 不再有 SIGWINCH (无 PTY): embed 侧写共享 size 槽 + `UiEvent::Resize`
  触发 autoresize。
- 原固定 100×30 (`COLS`/`ROWS` 常量) 已删除, 首帧量测前用 fallback。

## Agent 关闭的优雅退出

**当前选择 (2026-11-15 更新)**: 仍是硬杀, 但已序列化 — `shutdown()` 先发
`UiEvent::Shutdown` 让 embed 事件循环正常退出并 join, 线程尾部
`RpcClient::shutdown()` → `Transport::shutdown()` → `child.kill()` (SIGKILL)
pi 子进程; GUI 不再直接持有 child 句柄。

**为什么这样选**: MVP 阶段简化, 立刻释放资源; embed 线程自身干净退出,
只有 pi 子进程被强杀。

**spec 约束**: agent-shell "Manual session close" 现在写明 "Closing SHALL
**immediately** shut down the embedded session (terminating its pi
process)" — 立项优雅收尾须先开 spec delta。

**未来需要重新评估的信号**:
- pi agent 正在写文件 / 跑长任务时被强杀, 可能丢数据
- 用户期望 Ctrl+D / exit 能让 agent 自己收尾

**要做的事**:
- 关闭时先经 embed 注入通道送 `\x04` (Ctrl+D) 或 `exit\r` (不再是 pty 写)
- 等 N 秒看子进程是否自己退出 (监听 embed `is_running` / RPC 事件)
- 没退出再 fallback 到现有 SIGKILL 序列

## 键盘输入范围 (Ctrl 组合键)

**当前选择 (2026-11-15 更新: 已由 spec 定案)**: agent-shell 的 Keyboard
input forwarding 现在逐键列出可交付集合 — 可打印字符 (含 IME commit)、
Enter、Backspace、Tab、四个方向键、PageUp、PageDown; 并明确
**"Ctrl-combination keys SHALL NOT be delivered"**。滚轮 / 点击坐标是另外
的独立 requirement。面板侧 `handle_key_down` 对 control/platform/function
修饰键直接早退 (不转发也不拦截传播), 平台快捷键与 IME 菜单照常工作。

**为什么仍是这个选择**: 不再是 demo 简化, 而是 spec 边界 — Ctrl+C 中断、
Ctrl+R 翻历史等诉求被 spec 明确排除; 另外 Cmd+C 已被选区复制占用
(Terminal context 的 `TerminalCopy` 绑定)。注意 pi-shell 侧其实已实现一批
ctrl 快捷键 (ctrl+c 双击退出、ctrl+x/v 复制粘贴、ctrl+t/o 开关…), 但 GUI
的 control 早退使它们在 embed 路径下全部不可达。

**未来需要重新评估的信号 (立项时须开 spec delta)**:
- pi 生成长回复时, 用户想 Ctrl+C 中断
- 用户想 Ctrl+R 翻历史 / Ctrl+D 退出当前 session

**要做的事 (若立项)**:
- spec: Keyboard input forwarding 的 "Ctrl-combination keys SHALL NOT be
  delivered" 开明确例外 (列出允许的组合)
- 面板: `handle_key_down` 的 control 早退改为按白名单把 Ctrl+字母 映射成
  对应 ASCII 控制字符 (Ctrl+A=0x01 ... Ctrl+Z=0x1a) 并 stop_propagation
- 注意区分 Cmd+C (复制, 已占用) 和 Ctrl+C (中断), 取决于平台

## 统一 sidebar 事件处理器

**当前选择 (2026-11-15 更新, change `split-ui-modules-dockarea`)**: 事件漏斗 —
sidebar 渲染时经 `cx.listener` 构造 `SidebarEvent` 枚举 (6 种:
NewWorkspace / ToggleCollapse / CloseWorkspace / NewAgent / SelectAgent /
CloseAgent), 统一投进 `TerminalApp::on_sidebar_event` 单一入口 match 分发
(sidebar mod 头注释 design D3); 点击闭包本身不直接调业务方法。
不引入事件总线 / 命令模式的取舍不变 (理由同下)。

**未来需要重新评估的信号**:
- 事件种类继续膨胀 (现在已经 6 种), 中央 match 分发可读性下降
- 想给某些 action 加全局快捷键 / 撤销 / 重做
- 想做命令面板 (Ctrl+Shift+P 那种)

**要做的事**:
- 把每个 action 抽象成一个 `Command` trait 对象
- 在 app 层维护一个 `CommandQueue` 或 `EventBus`
- sidebar 只负责发出事件, 不直接触发状态变更

## rfd 文件夹选择 (原: 同步调用阻塞主线程)

**状态: 已解决** (2026-11-06, change `port-ui-to-gpui-kit`)。

- 同步版在 gpui 下会崩溃: 模态循环阻塞期间监听器仍持有 App 可变借用,
  50ms drain 循环的 `weak.update` 重入借用 → RefCell panic (实测)。
- 已切到 `rfd::AsyncFileDialog` + `cx.spawn`, Future 句柄存于 shell 的
  `_dialog_task` (design D8); 对话框打开期间主线程照常跑 drain 循环。
- 通则: gpui 事件监听器内禁止任何阻塞调用 (design D3)。
- 原记录的 "要做的事" (切 AsyncFileDialog + 存 Future 句柄) 已完成;
  gpui 原生 `prompt_for_paths` 仍是可选的后续替换 (D8)。

## 持久化路径选择 (项目本地 vs 用户目录)

**状态: 已解决** (change `single-workspace-project-model`)。

新形态不是“挪个位置的大文件”，而是按职责拆到两处:
- **workspace 配置**在工作区目录内 — `.balabala.workspace.json` (仅
  marker 身份; `file_tree` 规则区段已随过滤源迁到项目根 `.gitignore`
  退役 — file-filter-prune D9) + `balabala.solutions.json`；打开目录 =
  打开工作区，配置跟着数据走 (workspace-management/project-model spec)。
- **MRU 历史**在全局应用数据目录 (`directories` crate →
  `workspace_history.json`)；历史不属于任何工作区，放全局才不与
  “配置都在工作区里”冲突 (design D4)。
- legacy `.workspaces.json` 只读: 一次性播种历史 (`legacy_seeded` 标志);
  **永不改写**。原“按缺区段补拷规则”已随过滤源退役 (file-filter-prune)。

原“要做的事”全部落地; 唯一形态差异: 没有引入单一用户目录配置文件,
而是拆成 workspace 内 + 全局历史两件 (见 change design D3/D4)。

## 会话与折叠态不持久化

**状态**: 维持不持久化 — change `single-workspace-project-model` 落地时
对齐确认: agent 会话仍**不持久化** (重启即丢是既定语义；项目化后
稳定键 (project root) 虽已具备，恢复仍是独立后置项)；折叠态从
workspace 级改为 **per-project** (`collapsed_projects` 集合)，仍易失
不写盘。

**当前选择**: `.workspaces.json` (legacy, 只读) 只存 workspace 路径
列表 — 其中历史遗留的 `file_tree` 区段不再被读取 (规则源 = 项目根
`.gitignore`); agent 会话与展开/折叠态重启即丢 — spec: "Collapse state and
agent sessions SHALL NOT be persisted"。

**为什么这样选**: 会话恢复要么重开 `pi --mode rpc` 进程要么重放 RPC 状态,
成本高; 折叠态是纯 UI 细节。

**未来需要重新评估的信号**:
- 用户重启后抱怨 "我的 agent 全没了"
- 长任务会话被重启打断, 需要跨启动续接

**要做的事 (若立项)**:
- spec delta: persistence requirement 扩展存储内容 (每会话 = workspace +
  pi-N 名 + 状态; 至少恢复时重新 spawn)
- pi-shell-rs 侧: 新 spawn 即 `pi --mode rpc`, 历史能否接上取决于 pi 的
  会话恢复能力 (`/resume`)
- 写失败红条 / 保留非本区段条款 (workspace-management 已有) 对新字段同样
  适用

## IME preedit 在终端里的预览

**当前选择**: 只转发 `ImeEvent::Commit(s)` (上屏后的最终字符串),
不转发 `Preedit` (候选词预览) 和 `DeleteSurrounding` — 且已是 spec 条款
(Keyboard input forwarding: "Other IME events (preedit composition,
surrounding-text deletion) SHALL NOT be delivered"), 立项须开 spec delta。

**为什么这样选**: MVP 阶段先让中文能进会话, 预览是体验优化。

**未来需要重新评估的信号**:
- 用户抱怨打字时终端里看不到候选词预览, 体验跟原生终端差太多
- 想做光标位置感知 (DeleteSurrounding 要知道光标在哪)

**要做的事**:
- 在 vt100 网格底部 (或光标下方) 画一个临时预览行
- 监听 `Preedit { text, active_range_chars }` 事件, 实时更新预览
- Commit 时把预览行清掉, 把最终文本写到主网格
- DeleteSurrounding 要反查当前光标位置, 调 parser 删字符

## 一步切 pane focus (点 sidebar 直接拿焦点)

**状态: 已解决** (2026-11-06, change `port-ui-to-gpui-kit`)。

- 一步式模型落地: 点 sidebar agent 行 → 同时聚焦 `pane_focus:
  FocusHandle`, 键盘/IME 即刻生效, 无需再点面板。
- 聚焦态可视化: 终端面板框蓝 (聚焦) / 灰 (失焦)。
- specs delta 已同步 ("Sidebar selection grants keyboard and IME focus
  in one step")。
- 原两步方案 (egui `InteractiveFrame` + `memory.focused`) 已随 egui 删除。
- 仍待未来: keyboard-only 导航 — sidebar 尚未进 Tab 序, Tab 目前只在
  终端面板内被转发 (Terminal context 绑定)。

## Streaming 活动指示 (badge 视觉反馈)

**当前选择**: 不做 — 维持 spec 三态 (starting / running / exited)。

**背景 (2026-11-09, change `split-ui-modules-dockarea` 6.1 验收)**:
用户报告 "badge 在 streaming 期间没有实时变动"。诊断结论: **badge 无缺陷** —
- 创建后即绿: pi 首帧输出极快 (<100ms), 黄点 starting 一闪而过, 肉眼即"直接变绿";
- streaming 期间不变: running → running **本来就没有状态变化**, spec 无此态;
- quit 变红 (exited): 实时 ✓ (无头回归测试 `status_badge_refreshes_via_drain_loop`
  验证全链: drain → notify 转发 → 面板重渲)。

**未来需要重新评估的信号**:
- 用户明确想要 streaming 时的视觉反馈 (如"让绿点知道它在干活")
- 多 agent 并行时需要区分"哪个在流式输出"

**要做的事** (若立项):
- spec delta: agent-shell 的 status badge requirement 扩展一个活动态
  (或独立 capability), 定义视觉语义 (脉冲/闪烁/透明度呼吸)
- 实现点: 与 drain 循环联动 (有帧消费 -> 临时活动态 -> 超时回落 running)
- 注意: 活动态是**纯视觉**, 不改变三态语义与 quit 行为

**备选方案**:
- A: 绿点脉冲动画 (透明度呼吸, 有帧消费时启动, idle 停)
- B: 状态点旁小 spinner 字符 (网格内不可行, 只能 sidebar 侧)
- C: badge 数字/计数 (如本次会话 token 流量) — 依赖 pi 状态快照

## pi-shell-rs 不处理 Tab 键 (原生 TUI 功能在 embed 路径缺失)

**状态: 已解决** (2026-11-15 复验; 上游修复 2026-11-10)。

- 诊断 (2026-11-09, change `split-ui-modules-dockarea` 6.1 F3) 当时成立:
  菜单关闭态的 Tab 在 pi-shell-rs 无 handler, 注入后零反应; 应用侧全链
  已证实送达 (`TerminalTab` 动作 / CI `tab_forwards` 计数守卫 spec
  "Tab SHALL be delivered"), 责任确在上游。
- 上游修复: pi-shell-rs `add-path-tab-completion` (5ec2fee, 2026-11-10) —
  `key_intent` 把无修饰 Tab 分类为 `KeyIntent::ForcePath` → 打开路径补全
  菜单 (force 态); 菜单开着时 Tab 应用当前候选后关菜单。
- 复验 (2026-11-15, embed 注入): 空输入 + Tab → 路径补全菜单出现
  (候选列表 + `(1/16)` 计数), 整屏变化 ✓。
- 旧诊断里"零反应"另有一层无害解释: 输入行为 `你好好吗hi` 时路径前缀零
  候选 → 按 spec 静默 (零候选才静默关), 并非键未送达。
- 应用侧交付边界与测试守卫不变 (同上)。

**备选方案**: 原 A (pi-shell-rs 修复 Tab) 已完成; B (文档化已知限制) 作废。

## 文件树过滤的配置界面 (含 agent-shell 键盘条款修订) — 已消解

**状态 (2026-11-18, change `file-filter-prune`)**: **消解** — 过滤源
已从 workspace 配置改为项目级供给, 可输入的配置界面**永不立项**:

- 规则源 = 项目根 `.gitignore` (仅 git 管理的项目读取), 默认排除只藏
  `.` 开头 (六项名单撤销, node_modules/target 等默认可见); workspace
  的 `file_tree` 配置源、`load/set_file_tree_rules`、legacy 规则补拷、
  app 默认规则自举全部退役。
- 自定义过滤的唯一入口 = 编辑项目根 `.gitignore` + ⟳ 刷新重读
  (spec: asset-manager "Filter provisioning from the project root" /
  "Refresh re-enumerates in the background")。
- "即输即筛"信号随之作废; `agent-shell` 键盘例外 (过滤框持焦条款) 的
  立项前提消失 — 现行 "macOS 失焦是唯一不转发条件" 措辞保持不动。
- 右 dock header 的只读规则摘要位一并撤除 (dock-layout delta,
  file-filter-prune)。
- 求值语义 (目录行同等求值 / 枚举减支 / gotcha / 空目录行保留) 已由
  change `file-filter-prune` 的 asset-manager delta 钉住 (File-row
  filtering relative to project roots / Enumeration prunes excluded
  directories / default dot exclusions)。

## 文件树的自动刷新 (焦点重扫 / FS 监听)

**状态**: 当前选择已入 spec (file-tree "Refresh triggers and behavior":
首次加载 / 根路径变化 / 点击 ⟳, 后台扫描不阻塞交互、重读规则、保留展开态;
同 workspace 切 agent 不重扫是独立条款); 焦点重扫 / FS 监听仍待立项
(2026-11-09, change `right-dock-file-tree` 5.x)。

**当前选择**: 三个低频触发 — 首次加载 (根路径出现) / 切换活动 workspace
(根路径变化) / 手动 ⟳ 按钮; 同 workspace 切 agent 不重扫。

**未来需要重新评估的信号**:
- 用户频繁外部建文件后抱怨"要点刷新才能看到"
- 编辑器内改文件, 右树长期陈旧

**要做的事** (按成本排序):
- b: 窗口重获焦点时自动重扫 (零依赖, `did_become_key` 订阅 + 防抖)
- c: notify crate 实时监听 — 需防抖、目录移动事件归并、大树 diff
- 注意: 重扫 = 全量重建权威树, 展开态由面板集合贴回 (design D4) 不丢

## 文件树的惰性扫描 (大仓)

**状态**: 后台扫描已入 spec (file-tree "Refresh triggers and behavior");
默认排除已随 change `file-filter-prune` 改为 dot-only (asset-manager
"Module file enumeration with default dot exclusions"); 惰性扫描本身仍
待立项 (2026-11-09, change `right-dock-file-tree` design D6)。

**当前选择**: 全量递归 + dot-only 默认排除 (只藏 `.` 开头) + 项目根
`.gitignore` 规则 + 后台线程; node_modules/target/build/dist **默认可见**
(想藏就写 .gitignore)。

**未来需要重新评估的信号**:
- 无 `.gitignore` 的超大仓 (依赖/构建目录全量入树) 刷新明显卡顿
- ~~用户要求浏览被排除目录 (如按需展开 node_modules)~~ → **已消费**
  (2026-11-18, change `file-filter-prune`: 六项名单撤销, 依赖目录默认在树)

**要做的事**:
- 首展才扫该目录 (惰性) — 需解决两件事: `is_folder()` 对未扫目录为 false
  (空 children = 假文件, 面板 dir_ids 集合已兜图标, 但语义要补)、展开事件
  驱动的异步插入 + 树重建 (展开态靠面板集合保留)
- 或: 扫描深度上限 + 超限目录显示"展开以加载"

## 右键 "在 Finder 显示" 的跨平台

**状态**: 待立项 (2026-11-09, change `right-dock-file-tree` 4.2)。

**当前选择**: macOS only — `open -R <path>` 子进程; `cfg(not(target_os =
"macos"))` 分支为空操作。file-tree spec 场景措辞即 macOS ("Reveal opens
Finder with that file selected"), 跨平台化需同步改 spec。

**未来需要重新评估的信号**:
- app 要发到 Windows / Linux 用户手里

**要做的事**:
- Windows: `explorer /select,"<path>"`
- Linux: 桌面环境差异大 (nautilus -select 等), 需定义降级 (如打开父目录)
- 落地点: `reveal_in_finder` 的 cfg 分支 (src/tree/mod.rs)

## 右 dock 关闭后本会话无法恢复

**状态**: 待立项 (spec 已把它写成明确禁令, 立项须开 delta)。

**当前选择 (dock-layout spec "Right dock close")**: 关闭按钮把右 dock 收成
零宽, 中槽吸收空间; **本会话不提供任何重开控件**, 只有重启应用才恢复
(默认宽度 + 文件树面板)。

**为什么这样选**: demo 阶段少一个 toggle 状态面 (开关态不持久化)。

**未来需要重新评估的信号**:
- 用户误关后想找回文件树, 只能重启
- 未来做"显示偏好持久化" (dock 开关 / 宽度跨启动记住)

**要做的事 (若立项)**:
- spec delta: "Right dock close" 的 "SHALL NOT provide any control to
  reopen" 与 "Restart restores" 两条都要改
- 面板侧: header 或窗口边缘加 toggle; 开关态进 `.workspaces.json`
  (注意 workspace-management 的写失败 / 保区段条款)

## 中槽 viewer 的覆盖范围 (仅 rs / md)

**状态**: 待立项 (center-slot-multi-tab / md-viewer-toggle 定的 v1 边界)。

**当前选择**: 静态 handler 注册表只挂两个 — `.rs` → 只读代码查看器
(语法高亮, 不可改不可存), `.md` → 双模式查看器 (渲染态默认, 顶栏 toggle
切源码态; 模式不持久化, 重开回渲染态)。其他扩展名 (`.toml` / `.txt` /
图片…) 左键只有选中高亮, 无可见效果 (file-tree: no handler → no-op)。

**未来需要重新评估的信号**:
- 用户点 `.toml` / `.json` / `.txt` 期待预览
- 想看图片 / PDF / diff

**要做的事 (若立项)**:
- 在 `property_detail_manager::HANDLERS` 追加 handler (claims 按扩展名,
  按注册顺序第一个认领者胜出)
- 图片 / PDF 需要新面板类型 + dedup 表登记 (绝对路径为键, center-tabs
  dedup 条款)
- 同步 file-tree spec 的左键场景 (其 "v1: rs" 措辞与 "README.md 无 handler"
  例已落后于 md handler — md 查看器上线后 `.md` 即被认领, 2026-11-15 发现)

