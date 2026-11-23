# Tasks: solution-config-tab

## 1. SolutionConfig Tab 组件

- [x] 1.1 新建 `SolutionConfigPanel`（`PanelView` 形态：签题、可关、焦点
      句柄、双栏骨架），挂进中槽渲染
- [x] 1.2 path 文本输入 + browse 按钮（rfd picker 仅填充输入，不直连提交）
- [x] 1.3 预检三行（空 / 重名 = `create_is_noop` + 树内 project root /
      合法性 = exists 且 is_dir 单次 stat）与提交钮门控，数据读
      `service_layer.tree_view()` 快照；无 workspace 位置约束

## 2. 入口接线与生命周期

- [x] 2.1 `open_solution_config_tab`：固定 dedup key 登记 `open_tabs`、
      尾插 Center（console 后）、`on_removed` 摘除；`AddSolution` 事件改开
      Tab；再点入口走 select + 焦点回交既有分支
- [x] 2.2 退役 `add_solution` 的 rfd 直连提交链（`request_create_solution`
      缝保留）
- [x] 2.3 渲染测试：入口开签（index 1、无系统 picker 出现）、单例聚焦、
      关签后重开 fresh（specs/center-tabs 三场景对位）

## 3. 提交链与结果

- [x] 3.1 submit 走 `request_create_solution` → `Create` ask → 泵拉取；
      成功支：树镜像落地 + `sync_asset_registration` + **Tab 自关**；
      失败支：`OpError` 行内显示、Tab 保留
- [x] 3.2 关签后才到的失败回执补落 `app.notice`（在途/关签竞态防丢）
- [x] 3.3 测试：预检门控（非法路径与重名禁钮+失败行显示）、成功自关、
      关签零写入、外部 workspace 的 root 接受（specs/project-model 五场景
      对位）；既有 `request_create_solution` 直调测试保持绿

## 4. 回归与验收

- [x] 4.1 workspace 全量测试零回归；`cargo build --release` 零告警；
      新增代码 clippy 零告警
- [x] 4.2 headed 真机冒烟：入口 → Tab → 预检三行与钮门控 → 提交 →
      项目行出现且 Tab 自关；关签不写文件（用户执行后记录）
      — ✅ 2026-11-23 用户 headed 验收通过（两轮：首轮发现按钮/预检行
      仅挂 test_text 未绘字的空壳 bug → 双挂修复后复测 “ok 测试正常”）

## Workflow follow-up

- Review 通过后归档本 change，并验证归档结果。
