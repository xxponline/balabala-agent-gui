# Proposal

## Why

右 dock 至今是占位面板——`shell/placeholder.rs` 的注释明确写着"下个功能模块的接入样板，下个功能替换本文件即接入"。数据区的空槽已经留好，现在要填入第一个真功能：**从 workspace 派生的文件树浏览**（查看、过滤、打开、右键操作），并借此建立后续插件化的数据源体系 `workspace_property_source`。

## What Changes

- **右 dock 占位 → 文件树面板**（`src/tree/` 的 `TreePanel`）：header（标题 + 刷新按钮 + 只读过滤规则摘要 + 关闭 `×`）+ 树体，dock 机制（resize/注册/关闭）零改动
- **新增 `workspace_property_source` 模块**（`src/workspace_property_source/`）：
  - `WorkspacePropertySource` trait（dyn-safe，为插件化铺路）+ 静态 `SourceRegistry`（v1 仅注册文件树源）
  - 中立 DTO：`SourceNode`（树节点）/ `MenuDesc`（菜单描述），**不跨边界携带 gpui 类型**
  - `FileTreeSource` 首个实现：定位活动 workspace path → 排除表 → 后台全量扫描 → 权威树 + 过滤词解释 + 右键功能描述与执行 + 左键 open 钩子
- **树范围**：活动 agent 所属 workspace 的文件/目录树；无活跃 agent 显示空态
- **文件行过滤（忠实 gitignore 引擎）**：规则存于 `.workspaces.json` 新开的 `file_tree` 区段（按 workspace 绝对路径键控，值为模式行数组）；默认全显、通配符 `*`/`?`/`**`、`!` 反悔、后写覆盖先写；**只作用于文件行、目录恒显**（对 gitignore 的有意偏离）；v1 不在 dock 内打字，配置界面后续立项
- **左键**：触发 `open(item)` 钩子——v1 为 no-op，等"默认打开"新系统立项后接入
- **右键菜单 v1**：复制路径（绝对路径）、在 Finder 中显示（macOS）——由数据源描述、渲染、执行，端到端链路可验证
- **刷新时机**：首次加载 / 切换活动 workspace（根路径变化）/ 手动刷新按钮；扫描走后台线程 + 默认排除表
- **删除** `src/shell/placeholder.rs`，`src/shell/mod.rs` 换装 `TreePanel`
- **`dock-layout` 能力修订**：右槽默认渲染从"占位面板"改为"文件树面板"

## Capabilities

### New Capabilities
- `file-tree`: 右 dock 文件树浏览——活动 workspace 树的显示、后缀过滤、左右键交互、刷新时机、空态与降级表现

### Modified Capabilities
- `dock-layout`: "Right dock renders by default" 的渲染内容由占位面板改为文件树面板（chrome 从"标题+×"扩展为"标题+刷新+只读过滤摘要+×"）
- `workspace-management`: "Workspace persistence to JSON file" 的文件格式从"单一 `workspaces` 键"扩为"可携带其他区段"，任何写入须保全 `file_tree` 等非本次变更所拥有的区段

## Impact

- **新增**：`src/workspace_property_source/{mod.rs,filetree.rs}`、`src/tree/mod.rs`
- **删除**：`src/shell/placeholder.rs`
- **修改**：`src/shell/mod.rs`（换装）、`src/app.rs`（面板转发/通知接缝，预计小改）、`src/workspace/store.rs`（`file_tree` 区段读取 + 写入保全）
- **依赖**：零新增 crate（剪贴板走 gpui，reveal 走系统命令）
- **测试**：现有 14+1 回归保持绿；新增扫描/过滤/分发的无头测试
