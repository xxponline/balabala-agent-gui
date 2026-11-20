# Proposal

## Why

主骨架今天允许"同服务多实例"（`service-lifecycle` 明文规定同名 N 实例共享
dlopen，`service-routing` 的 `SelectPolicy::Any` 负责挑一个），但负载均衡与
分客户处理从未实现——`route.rs` 自己写着"round-robin 等真有负载需求再加"。
多实例唯一留下的遗产是一套永远需要回答"挑哪个"的寻址词汇和更模糊的实例表。

方向已定：**主骨架下任何一类 Server 恰好一个实例，类 = 契约身份
（TypeId）**；需要多实例做负载均衡或分客户处理时，由单独的**容器型服务**
承担——容器在骨架里同样只占一个坑，N 个子实例是它的私有实现，永不进骨架
实例表。整个系统从"名字 + 挑选策略 + N 实例"收敛为"契约 → 唯一实例"，
更清晰、可推理。

## What Changes

- **BREAKING** 骨架级单实例强制：`load()` 仅当该契约在实例表中无实例时
  成功；同名二次装载、同契约异名装载一律**显式拒载**
  （新错误 `LoadError::ContractAlreadyLoaded`，携带占用方与请求方名字）；
  已存在的实例任何状态（含 `error`）都占位，需 `retry()`/删除后才释放槽位。
- **契约身份贯通**（今天 descriptor 只有 `name/origin/meta`，无契约身份）：
  新增 `ContractId`（宿主侧 marker 类型的 `TypeId` 铸造，永不跨 dylib 传值）；
  内部服务注册显式携带；DLL 产物新增契约导出符号（沿用指纹符号的既有模式），
  装载时由宿主注册表解析，未知/缺失 → 显式拒载（与指纹同哲学，绝不静默）。
- **BREAKING** 寻址简化：`Route::Service(name, policy)` →
  `Route::Service(name)`，**删除 `SelectPolicy`（`Direct`/`Any`）**；
  服务名寻址 = 直呼该契约唯一实例，"实例挑选"概念从契约面消失。
  （⚠ 假设待复核：此条为探索中推荐方案，用户未逐字确认——见 design 决策 D3。）
- **容器约定（零框架代码）**：容器 = 普通服务内部私有多路复用；子状态经
  容器自己的投影消息上报（`SessionsChanged{SessionView}` 既有先例）；
  框架不新增容器机制——约定写入 design，不新增规格行为。
- 规格 delta：`service-lifecycle`（多实例装载 → 每契约单实例 + 契约身份
  声明）、`service-routing`（两级寻址去选择策略）。
- 现有多实例测试（`task_3_3_multi_instance_internal_service`、
  `instances_of.len()==2`、`pick_available` 族）改写为拒载/解析语义。

## Capabilities

### New Capabilities

（无——容器是约定而非新行为；契约身份与拒载语义归入既有生命周期能力。）

### Modified Capabilities

- `service-lifecycle`: "Loading and multi-instance creation" → 每契约恰一
  实例、二次装载显式拒载、共享 dlopen 段落随 N 实例消失；并新增契约身份
  声明要求（descriptor 携带 / DLL 符号解析 / 未知契约显式拒载）。
- `service-routing`: "Two-level addressing" 删除 `Direct`/`Any` 选择策略，
  服务名寻址解析到该契约唯一实例。

## Impact

- `crates/service-core`: `route.rs`（`Route::Service` 单形态、删
  `SelectPolicy`）、新增 `ContractId`。
- `crates/service-manager`: `loader`（装载前唯一性检查 + 错误变体）、
  `instance`（Participant 携带 contract、按契约索引）、`router`（解析分支
  收敛）、`catalog`（descriptor 契约字段/宿主注册表 API）、`flow_tests`。
- `crates/service-plugin`: 脚手架生成契约符号文件（仿 `FINGERPRINT_RS`）。
- `contracts` / `crates/agent-service`: 各自契约 marker 类型。
- `src/app.rs`: `register_internal` 签名 +1 契约参数、10 处
  `SelectPolicy::Any` 调用点；`src/service_layer*` 测试。
- **风险**：`service-core` 枚举形状变更 ⇒ 全部插件产物必须重编
  （fingerprint 目前不覆盖 service-core 源码的既有盲区——见 design Risks）。
- 不动：信封/ask-tell/邮箱/fail-fast/监督恢复（六条路由规格中五条原样、
  七条生命周期规格中五条原样）。
