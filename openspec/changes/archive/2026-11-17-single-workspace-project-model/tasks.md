# Tasks

## 1. 领域模型层 (design D1–D3, project-model spec)

- [x] 1.1 新建 `src/project_model/`：`SolutionItem`/`SolutionType` 注册表与
  `SimpleSolution` 展开为 `Project { name, root, modules }`；
  单测断言 1 个 `simple` item → 1 个 Project + 同名 root Module、2 个 item →
  2 个 Project —— 验证: `cargo test project_model` 通过
- [x] 1.2 实现 `validate::load()` 错误契约（解析失败 / 未知 SolutionType /
  path 不存在 / DAG 环）返回 `ModelError`；单测覆盖每个错误变体 +
  合成 A↔B 环输出被拒 —— 验证: `cargo test` 通过
- [x] 1.3 实现 workspace 文件契约 IO：marker `{"version": 1}` 与 solutions
  文件读取，创建顺序 = 先 solutions 后 marker；临时目录测试断言写失败后
  无 marker 残留（不存在半工作区）—— 验证: `cargo test` 通过
- [x] 1.4 实现 solution 追加写入（新增 `simple` item、保留既有 items）与
  文件缺失/空数组语义（缺失 = 合法空工作区）；roundtrip 单测 ——
  验证: `cargo test` 通过

## 2. 历史存储与 legacy 迁移 (design D4, workspace-management spec)

- [x] 2.1 添加 `directories` 依赖，实现全局 `workspace_history.json`
  （去重、封顶、`legacy_seeded` 标志）；单测覆盖最近序、封顶、
  历史文件损坏时按无历史处理 —— 验证: `cargo build` 与 `cargo test` 通过
- [x] 2.2 实现 legacy `.workspaces.json` 一次性迁移：历史播种（写
  `legacy_seeded` 后不再读）+ 打开时按缺区段补拷 `file_tree` 规则；
  临时目录测试断言 legacy 字节永不改变、规则仅在缺区段时拷贝 ——
  验证: `cargo test` 通过
- [x] 2.3 更新 `Docs/pending-decisions.md`：「持久化路径选择」「会话与
  折叠态不持久化」条目标记为本 change 结案/确认 ——
  验证: 文档条目含 change 名 `single-workspace-project-model` 引用

## 3. 启动恢复与空态 (design D5, workspace-management ADDED)

- [x] 3.1 启动接线：主窗先行 → 有历史则异步 load 最近工作区，带 epoch
  防竞态；坏 marker → 错误框 + 退出；无头测试覆盖恢复成功与坏 marker
  错误路径 —— 验证: `cargo test` 通过
- [x] 3.2 历史无档 / 目录与 marker 缺失 → 窗内空态 + 目录选择器
  （`rfd::AsyncFileDialog` + `cx.spawn`，D8 先例）；无头测试断言空态分支
  且不弹错误框 —— 验证: `cargo test` 通过
- [x] 3.3 新建工作区确认流：确认 → 写两文件 → 打开；拒绝 → 留空态；
  临时目录端到端测试 —— 验证: `cargo test` 通过

## 4. 切换门控 (design D6, workspace-management ADDED)

- [x] 4.1 `SidebarEvent` 扩 `SwitchWorkspace` 等载荷，漏斗入口实现三支
  门控（活动拦截 Notice / 仅 exited 确认 / 无会话直通）；无头测试三支
  各一 —— 验证: `cargo test` 通过
- [x] 4.2 切换执行：确认后丢弃会话、epoch++、复用启动加载路径；
  竞态测试：load 未完成时切区，断言旧结果被丢弃 ——
  验证: `cargo test` 通过

## 5. 左槽 Project 树 (design D7–D8, agent-shell deltas)

- [x] 5.1 会话接线：cwd 改读 `project.root`（`AgentEntity` 暴露语义更名）、
  计数器移入 Project；单测：cwd = project root（项目可在工作区外）、
  计数器 per-project 独立 —— 验证: `cargo test` 通过
- [x] 5.2 左槽三段式重写（switch entry / project 行 / `[+ agent]` /
  add-solution / 空态），折叠态 `HashSet<PathBuf>` 易失存储；测试：
  无 project 无按钮、折叠隐藏 agent 行、点 project 行不切 active tab ——
  验证: `cargo test` 通过
- [x] 5.3 add-solution 流程：选择器 → 追加 item → project 行不重启出现；
  取消不写文件；临时工作区测试 —— 验证: `cargo test` 通过
- [x] 5.4 console 状态栏改为 project root 路径
  （`/Users/me/code/foo — pi-1 [Running]`）；测试断言格式 ——
  验证: `cargo test` 通过
- [x] 5.5 更新 `README.md` 架构与功能描述（三槽说明、sidebar → project
  树、workspace 生命周期）—— 验证: README 对应段落与新行为一致

## 6. 右槽规则来源 (design D9, file-tree delta)

- [x] 6.1 过滤规则读取从 app 根 store 移到活动工作区
  `.balabala.workspace.json`；测试：从工作区配置读规则、⟳ 重读生效 ——
  验证: `cargo test` 通过
- [x] 6.2 移除 app 根 `.workspaces.json` 写路径（仅剩 legacy 只读入口）；
  验证: `grep -rn "workspaces.json" src` 只命中 legacy 迁移代码，且
  `cargo test` 全绿

## 7. 集成验证

- [x] 7.1 全链路无头测试：创建工作区 → 添加 solution → 项目行出现 →
  建 agent（cwd 正确）→ 门控切换；验证: `cargo test` 全套通过
- [x] 7.2 `openspec validate "single-workspace-project-model" --strict`
  保持通过 + `cargo run` 手工冒烟（启动空态 / 建区 / 坏文件报错文案可读）

## Workflow follow-up

- 归档 change（`openspec archive`）并同步主 specs。
- 归档后核对主 spec 内容与 git 历史。
