# Tasks

## 1. 契约身份与宿主注册表

- [x] 1.1 `service-core` 新增 `ContractId`（`TypeId` 新类型，Copy/Eq/Hash/Debug，中文文档注释）——验证：`cargo test -p service-core` 中新增的铸造/相等/哈希单测通过
- [x] 1.2 契约 marker 类型：`contracts::demo` 增 demo marker、`agent-service` 契约模块增 agent marker——验证：workspace 编译通过，单测断言两 marker `TypeId` 互异
- [x] 1.3 `ServiceDescriptor` 与 `Participant` 携带 `contract` 字段，`Instances` 维护 `by_contract` 索引（insert 唯一断言）——验证：`instance.rs` 新单测覆盖索引建立、同契约二次 insert 断言、卸载后索引清除
- [x] 1.4 宿主注册表 API `register_contract(name, ContractId)`（同名不同 marker → debug 断言）+ `register_internal` 签名增契约参数 + 引导处登记 demo/agent——验证：注册表单测通过，`cargo test --workspace` 编译全绿（含既有调用点改写）

## 2. 装载拒载门与契约符号解析

- [x] 2.1 插件脚手架生成契约符号文件（仿 `FINGERPRINT_RS` 模式，`example-plugin` 声明 `"demo"`）——验证：脚手架生成单测断言符号名一致，构建出的 example-plugin 产物含该符号（load 流程可读）
- [x] 2.2 `load()` 装载期解析契约：读符号 → 宿主注册表查询；缺符号复用 `MissingSymbol`、未知名新增 `LoadError::UnknownContract`——验证：两个拒载单测（缺符号产物、未知契约名）均断言显式错误且无实例创建
- [x] 2.3 拒载门：装载前查 `by_contract` 占用（**任何状态含 `error` 都占位**），占用即 `LoadError::ContractAlreadyLoaded {loaded_name, requested_name}`；`unload()` 释放槽位——验证：三条 spec 场景单测：同名二次装载拒载、同契约异名装载拒载、卸载后重新可装
- [x] 2.4 内部服务与多契约共存走同一装载流：internal 装载携契约进表；两个不同契约实例并存——验证：spec 场景 "Internal service declares its contract" 与 "Distinct contracts load side by side" 单测通过
- [x] 2.5 改写既有多实例测试：`task_3_3_multi_instance_internal_service` 及共享 dlopen 的 N 实例断言改为拒载语义——验证：`cargo test -p service-manager` 全绿，且 `rg "同服务两实例|instances_of.*== 2" crates/` 无旧语义残留

## 3. 寻址契约面收敛（D3 决策门）

- [x] 3.1 ⚠ 决策门：向用户确认 design D3（删除 `SelectPolicy`，`Route::Service(name)` 单形态）——验证：获得用户明确答复；若改回"保留策略"，本组 3.2–3.4 替换为轻改版（语义退化 + req2 delta 回退为 MODIFIED 轻改），其余组不受影响
- [x] 3.2 `service-core::route` 删除 `SelectPolicy`、`Route::Service(ServiceName)` 单形态，`route.rs` 单测改写——验证：`cargo test -p service-core` 通过
- [x] 3.3 `router` 名寻址分支收敛（删 `Direct` 归属断言与 `Any` 挑选 → 按名取唯一实例，fail-fast 语义保持）——验证：新单测覆盖 spec 场景 "Service name resolves to the unique instance" 与 "Service name with no loaded instance fails fast"，`cargo test -p service-manager` 通过
- [x] 3.4 调用点机械改写：`src/app.rs` 10 处、`src/service_layer*`、其余测试——验证：`rg "SelectPolicy" src/ crates/` 零命中，`cargo build --workspace` 通过

## 4. 集成验证

- [x] 4.1 插件产物全量重编后跑通全部 DLL 流程测试（堵 `service-core` 枚举变更与旧 dylib 的 ABI 失配，design Risks 第一条）——验证：`cargo test -p service-manager`（含 flow_tests 5.x DLL 端到端）全绿
- [x] 4.2 全仓基线对账：`cargo test --workspace` 全绿、0 warnings，测试数 ≥253 且新增数对得上（拒载≥3、契约身份≥3、寻址≥2）——验证：命令输出与计数记录在提交说明
- [x] 4.3 规格自查：`openspec validate singleton-per-contract --strict` 通过；两个 delta 的全部 scenario 均有对应测试——验证：命令输出 valid，逐 scenario 对照任务 2.3/2.4/3.3 的测试名
- [x] 4.4 design.md 复核点收口：D3 的用户答复与最终形态写回 design Decisions——验证：文件中 ⚠ 标记替换为确认结论（或回退记录）

## Workflow follow-up

- 全部任务完成后走 `/opsx-archive`：delta 合入主规格（`service-lifecycle` 7→6+2、`service-routing` 6→5+1）、归档、`docs(openspec):` 提交。
