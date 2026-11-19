# Proposal

## Why

应用的核心模块（AgentEntity、AssetManager、各类面板后端）目前以散装形态直接
长在 GUI 进程里，模块之间靠进程内直接调用耦合，没有统一的装载、状态与通信
机制。本 change 引入一个**可独立编译、独立被引用的服务管理中间层**：它扫描
并加载服务（内部 crate 或外围 DLL）、管理多实例与标准状态（offline/loading/
available/error）、并让 GUI 本体与所有服务通过统一路由互相通信。这是后续把
现有模块逐个服务化的骨架；骨架先立，模块迁移是后续独立的 change。

## What Changes

- 新增 workspace 级中间层三件套 crate：`service-core`（词汇表：
  Route/Envelope/Payload/ServiceState/RouteError）、`service-manager`
  （Router/Scanner/Loader/Supervisor + 自建 tokio 运行时）、`service-plugin`
  （插件侧 ingress/actor 脚手架）。三者对 GUI 与具体服务契约零依赖。
- 新增 `contracts` crate（按服务分目录、单 crate 起步、预留拆分形状）：
  服务间互相认识的类型化 Message 定义（**消息归调用方**、双向共享类型归
  中立命名空间，保证将来可无循环裂分），Router 不依赖它。
- 服务装载：启动时扫描服务目录 + 显式 `reload_catalog()`（无文件系统
  watch，reload 只影响未来 load、不动实例表）；内部服务经同一契约同一
  注册流程（仅"符号来源"不同）；同一服务多实例 = 一次 dlopen + 多次实例
  创建；DLL 产物带构建指纹，装载前校验（契约构建不一致即拒载）。
- 每实例标准状态机：`loading / available / offline（主动停用）/ error`，
  未 load 的服务不存在于实例表；状态翻转以事件推送宿主；**恢复唯一走
  状态机**（crash → error，重建只能经显式 retry，kameo 自动 restart 禁用）。
- 中心路由：全部消息（宿主 actor、GUI、插件 actor）走统一信封过 Router；
  两级寻址 `Route::Instance(..)` 与 `Route::Service(name, Direct|Any)`；
  payload 默认类型化（`Arc` 透传，不序列化），序列化封装是显式的消息类型；
  ask 的回复与超时同样走总线（Router 持 deadline）；**fail-fast 零缓冲**：
  目标非 available 时立即拒绝、投递失败（含队列满）同样即刻失败（类型化
  错误），死信可观测；用户触发操作一律走 ask（tell 无同步反馈）。
- 中间层自建并自持 tokio 运行时，对外 API 不泄漏 tokio：返回的 future
  executor-agnostic（内部急切 spawn + oneshot 收口）；非 async 调用方
  （gpui）作为**拉模式参与者**持自己的 mailbox（Listener）经 `poll()`
  取件——**无回调 API**，跨线程回调风险从结构上消除。
- 工程结构：根 `Cargo.toml` 改造为 workspace；GUI 现有模块本 change **不
  迁移**，仅作为框架的第一个消费方接入。
- **BREAKING**：`Cargo.toml` 由单 package 变 workspace（构建入口变化，
  `cargo build` 行为不变）。

## Capabilities

### New Capabilities

- `service-lifecycle`: 可加载服务的目录扫描与 Catalog、装载/停用/重载、
  内部服务与 DLL 服务的统一登记、同服务多实例、每实例标准状态机及其事件
  语义。
- `service-routing`: 中心路由的信封与类型化 payload、两级寻址与实例挑选
  策略、请求/应答与超时、fail-fast 错误与死信观测、宿主/GUI/插件参与者
  对等。

### Modified Capabilities

（无 —— 现有 specs 的需求均不因本 change 改变；既有模块服务化属后续
change。）

## Impact

- **新增代码**：`crates/service-core`、`crates/service-manager`、
  `crates/service-plugin`、`crates/contracts`、示例插件 cdylib
  （`plugins/`，兼作边界集成测试载体）。
- **改造**：根 `Cargo.toml` → workspace；`src/main.rs` 所在 package 成为
  workspace 成员。
- **依赖**：`kameo`（已加入，本 change 起实际使用）、`libloading`（新增）、
  tokio（经 kameo 引入，仅中间层内部使用）。
- **不受影响**：`openspec/specs/` 现有 8 个 capability 的需求、现有 GUI
  行为与测试。
- **风险/验证点**（实现期 spike）：类型化 payload 跨 cdylib 边界的
  `TypeId` 一致性（构建指纹校验 + dlopen round-trip 测试钉死）、`unload`
  的 dlopen 句柄承诺、运行时拓扑的线程/事件桥。
