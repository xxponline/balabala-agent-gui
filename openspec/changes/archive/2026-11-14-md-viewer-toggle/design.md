# Design

## Context

- default-open 后端（`property_detail_manager`）已按"认领表"分发：首个 claims 的 handler 开 tab；现只有 `claims_rust`（`.rs` → `EditorPanel`），md 落在 unclaimed → 打开无动作
- `EditorPanel` 是现成的只读 Editor 模板（`BasePanel + ComponentPanel + Focusable` 三件套、`PanelEvent`、焦点 = EditorState 的 focus handle）
- tab 条 UI 有两种形态：单签 30px 标题栏 / 多签 tab 条（center-slot change 的方案 B），切换按钮两种形态都要有落点
- 渲染侧资产已验证（见下）：gpui-component `TextView::markdown(id, text)`
- **TextView 静态渲染验证结论（源码三重证据）**：
  1. `TextViewMotion::default()` 三个 duration 全零——"an unstyled view adopts new content at once"
  2. 全量替换路径立即显示（`set_text` 文档 + 测试 `set_text_replacing_the_text_shows_it_at_once` 钉死）；淡入只对 `push_str` 流式追加生效，我们的构造时全文走 replacement 路径
  3. 保险开关存在：`.stream_fade(false)` 关 theme 时序、`.motion(...)` 显式覆盖
  - 附带事实：`MAX_SYNC_FULL_REPLACE_BYTES = 4KB`——超此尺寸走后台解析，首帧可能一瞬空缺（非动画，可接受）；自由函数版 id 是 `CodeLocation`，须用 `TextView::markdown(id, text)` 传稳定 id（tab object key）

## Goals / Non-Goals

**Goals:**
- md 文件双模式只读查看：默认渲染、tab 条按钮一键切换、不记忆
- 源码模式复用既有 Editor 管线（syntect markdown 语法）
- 认领表扩展一行 + 新面板自包含，零改动 tab 系统与 EditorPanel

**Non-Goals:**
- WYSIWYG 编辑、任何写回磁盘
- 渲染↔原文的同步滚动、位置对应（切换式正是为回避它）
- `MarkdownPlugin` 节点扩展（frontmatter 折叠、代码块复制按钮等留 v2）
- 模式记忆（全局/per-file 均不做）
- 双击选词、链接点击等渲染视图交互（纯静态展示；链接不可点 v1 接受）

## Decisions

### D1 单面板双模式，不拆两种 panel

`MdPanel` 持 `view_mode: enum { Rendered, Source }`，切换 = 改 enum + `notify`。
备选"两种 panel 类型共享 tab"被否：tab 系统需感知"同一对象两种呈现"，
动 dedup（absolute path 识别）与 tab 身份模型，违背 Non-Goal 的零 tab 改动。

### D2 切换按钮挂在 tab 条，按 panel 类型条件显示

按钮只在 active tab 是 md viewer 时渲染（spec 场景 3 钉住 console/rs 无按钮）。
两种 tab 形态（标题栏 / tab 条）各留一个条件渲染点，复用既有 title_bar 机制。
切换 action 走 `actions!` 注册（与 TerminalTab 同模式），按钮与快捷键共用同一 action。

### D3 渲染态持有 TextView state，源码态持有 EditorState

两态的重量级 state（`Entity<TextViewState>` / `Entity<EditorState>`）在 panel
构造时都建好，按 mode 挂载其一——切换不重建 state、不重读文件。
备选"切换时重建"被否：每次切换 IO + 解析，且 source↔rendered 来回会丢
Editor 光标位置（只读场景光标是浏览位置，值得保留）。

### D4 焦点 = 当前 mode 的可聚焦者

`Focusable` 返回：Rendered → 面板自身 focus handle（TextView 只读浏览）；
Source → EditorState focus handle（与 EditorPanel 同语义）。
tab 切换的 `focus_active_panel` 不用改——它调面板的 focus handle，
MdPanel 内部按 mode 转发即可。

### D5 文件内容读一次、两态共享

`content: String` 在 open 时读入，Rendered（灌 TextView）与 Source（灌 Editor）
消费同一份。打开失败 → 空 content（与 EditorPanel"打开不失败"一致）。

### D6 认领与语言名

`claims_md(obj)` = 扩展名 `md`；Editor 语言名 `"markdown"`（syntect 语法定义
名）。认领表注释"v1: 仅 rs 认领"同步更新为 rs|md。

## Risks / Trade-offs

| 风险 | 评估 | 应对 |
| --- | --- | --- |
| TextView 对大文件首帧延迟 | >4KB 走后台 parse，可能一瞬空缺 | v1 接受；README 量级无感；真问题再议增量 |
| 渲染视图无动画验证仅靠源码 | 已三重证据 + 保险行 | 实现时构造侧带 `.stream_fade(false)` |
| 两态 state 常驻内存 | md 文件小，双 state 成本可忽略 | 无需策略 |
| tab 条按钮在两形态下的布局 | title_bar 与 tab 条是两处 UI | 任务里分列，各一个条件渲染点 |

## Open Questions

（无）
