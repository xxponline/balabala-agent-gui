# Tasks

## 1. MdPanel 面板骨架（双模式 state）

- [x] 1.1 `MdPanel` 结构：`BasePanel + ComponentPanel + Focusable` 三件套照 EditorPanel 模式；字段 `content: String`、`view_mode: Rendered | Source`（默认 Rendered）、`Entity<TextViewState>` 与 `Entity<EditorState>` 构造期都建好（D3/D5）
- [x] 1.2 内容灌入：open 时读文件一次（失败 → 空 content，打开不失败）；Rendered 灌 `TextView::markdown(stable_id, content)` + `.stream_fade(false)` 保险行（D5/验证结论）
- [x] 1.3 双模式渲染分支：Rendered 挂 TextView、Source 挂 Editor（语言名 `"markdown"`）；焦点按 mode 转发（D4）；纯测：默认 Rendered、两态 state 各自存在

## 2. 认领接入 default-open

- [x] 2.1 `claims_md`（扩展名 `md`）注册进 handler 认领表；`claims_rust` 旁"v1 仅 rs"注释同步更新；测试：md 对象开 MdPanel tab（标题为文件名）、rs 行为不变、unclaimed 仍无动作
- [x] 2.2 dedup 回归：同一 md 文件开两次仍单 tab（absolute path 识别不因新 handler 失效）

## 3. 切换按钮与 action

- [x] 3.1 `actions!` 注册 toggle action；action 只对 MdPanel 生效（改 `view_mode` + notify），console/rs 面板无此 action 响应
- [x] 3.2 切换按钮 UI：tab 条与单签标题栏两种形态各一个条件渲染点（仅 active tab 为 md viewer 时显示）；点击调同一 action；测试：渲染↔源码原位切换、tab 不重建位置不变、console 与 rs 无按钮

## 4. 行为契约测试（spec 场景逐条）

- [x] 4.1 `md file opens rendered`（默认渲染视图）、`Source mode shows raw markdown highlighted`（原文 + 高亮路径）、`Viewer is read-only`（两模式键入无变化）
- [x] 4.2 `Reopen resets to rendered`（关 tab 重开回渲染——不记忆）、`Toggle switches representation in place`（无第二 tab、保持 active）
- [x] 4.3 只读回归：rs viewer 行为逐字不变（既有测试全绿）

## 5. 回归与验收

- [x] 5.1 全量 `cargo test` 绿、`cargo check --all-targets` 0 warning、`openspec validate md-viewer-toggle --strict`
- [x] 5.2 headed 真机验收：md 打开即渲染（标题/列表/代码块排版正确）、按钮切换手感、关开重置、rs/console 无按钮、大 README 首帧可接受
