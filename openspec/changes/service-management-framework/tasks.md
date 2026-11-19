# Tasks

## 1. workspace 化（纯结构，可独立 revert）

- [ ] 1.1 根 `Cargo.toml` 改为 workspace（含原 package 作为成员，`src/`
      原位保留），新增 `crates/`、`plugins/` 目录骨架（`plugins/` 产物
      gitignore）。验证：`cargo build` 与 `cargo test` 全绿，行为与改造前
      一致；单独一个提交。
- [ ] 1.2 依赖边落位：GUI 成员依赖 `contracts` + `service-manager`；
      `service-manager` 只依赖 `service-core`（不得依赖 contracts/GUI）；
      `libloading` 入 `service-manager`。验证：`cargo tree` 检查无反向边；
      workspace 全量构建通过。

## 2. service-core + contracts（词汇表）

- [ ] 2.1 `service-core`：`Route`（Instance / Service(name, Direct(id)|Any)）、
      `Envelope`（to/from/corr/kind/payload，from 仅实例/actor 地址）、
      `Payload::Typed(Arc<dyn Any + Send + Sync>)`、`ServiceState`
      （loading/available/offline/error）、`RouteError`（Timeout /
      TargetNotAvailable / DeadLetter / NoSuchRoute / TypeMismatch）。
      验证：单测覆盖各类型构造与语义断言（如 from 拒绝 Service(..)）。
- [ ] 2.2 `contracts` crate（零依赖）：命名空间骨架 `contracts::shared`
      （双向共享类型）+ 按服务分目录；归属规则注释写明"消息归调用方、
      双向归中立"。验证：`cargo build -p contracts` 零依赖通过。

## 3. service-manager：Catalog / Loader / Supervisor

- [ ] 3.1 Scanner + Catalog：启动扫服务目录建表；内部服务显式注册与 DLL
      条目同表（`ServiceDescriptor { name, origin: Internal(fn)|Dll(path),
      meta }`）；`reload_catalog()` 只影响未来 load、不动实例表、同名新条目
      替换旧条目。验证：单测——扫描建表、内部/DLL 同表、reload 替换条目但
      在跑实例不受影响、已删条目拒绝新 load。
- [ ] 3.2 构建指纹：`service-plugin` 提供 build.rs 模板嵌入指纹（rustc
      版本 + 契约元数据哈希）并导出固定符号；Loader 在 create 前比对宿主
      指纹，不匹配显式拒载（落 error/诊断）。验证：单测——指纹一致通过、
      篡改指纹的产物被拒且报错信息含 mismatch。
- [ ] 3.3 Loader：load()/unload() 流程按 origin 分支取符号；同一 DLL 多
      实例 = 1 次 dlopen + N 次创建（`Arc<Library>` 共享）；unload = 停用 +
      移出实例表、**不 dlclose**，API 文档明示"停用"语义。验证：集成测试
      （示例插件）——两实例同库句柄；unload 后实例表无该 id 且库仍驻留。
- [ ] 3.4 Supervisor 状态机：每实例状态与全部转移（load→loading、init
      ok→available、init 失败→error、disable→offline、enable→loading、
      crash→error、retry→loading）；删实例仅限 error；**禁用 kameo 自动
      restart**（on_failure 只上报）。验证：单测覆盖每个转移 + "crash 后
      不自动拉起、消息 fail-fast"。

## 4. Router / 运行时 / mailbox

- [ ] 4.1 自建 tokio 运行时 + executor-agnostic 的 call()（内部急切 spawn
      + oneshot 收口）；对外 API 不暴露 tokio/kameo 类型（宿主侧）。
      验证：单测在非 tokio executor（block_on / 线程池）上 await call()
      得到结果。
- [ ] 4.2 Router：统一信封转发、两级寻址（Direct 的服务名归属断言、Any
      挑 available）、per-outlet 有界通道（try_send，满即刻销账）。
      验证：单测——Instance 直投、Direct 错服务名被拒、Any 挑中 available、
      队列满即刻 `Err(DeadLetter(ChannelFull))`。
- [ ] 4.3 ask/回复/超时：corr 销账（不按 from）、Router 持 deadline、超时
      落死信并 `Err(Timeout)`；fail-fast（ask `Err(TargetNotAvailable)`、
      tell 落死信）。验证：单测——一问一答、超时、目标 loading/error 时
      即刻拒绝、全链路留痕可查。
- [ ] 4.4 Listener（拉模式 actor mailbox）：`submit(corr)`/`poll()`/
      `drain()`/`snapshot()`，同步、调用方线程执行；变更事件自带快照 Arc；
      无任何回调 API。验证：单测——poll 取件、snapshot() 零回程、连续 N 次
      状态翻转可合并为一次 changed、Reply 事件不合并。

## 5. service-plugin + 示例插件（D12 边界载体）

- [ ] 5.1 `service-plugin`：插件侧 ingress/actor 脚手架 + extern 入口模板
      （typed extern fn 递 `&Msg` 的形状按 D12 路线乙预留接口）。
      验证：示例插件 cdylib 编译通过并可被扫描装载。
- [ ] 5.2 D12 钉死测试：真 dlopen 示例插件后 round-trip 一个 Typed
      payload 并断言 downcast 成功；另加指纹不匹配拒载用例。
      验证：`cargo test` 集成测试绿（含 `plugins/` 产物构建脚本）。

## 6. GUI 接入（本 change 最后一步）

- [ ] 6.1 GUI 注册为总线参与者（自己的 Route + mailbox），启动装载核心假
      服务/示例服务并订阅状态翻转；Listener 接入 gpui 泵（并入现有
      drain/notify 循环，`cx.defer` 后 `entity.notify()`）。验证：手测——
      服务状态翻转驱动界面刷新；GUI 触发操作一律走 ask、错误可回显。
- [ ] 6.2 验收边界守住：本 step 只保证订阅/mailbox poll 通路可用，不实现
      服务状态页 UX（留后续 GUI spec change）、不迁移任何现有模块。
      验证：现有测试全绿，GUI 行为无回归。

## 7. 文档与承诺收口

- [ ] 7.1 API 文档写明三条承诺：unload 不 dlclose（真回收=进程重启）；
      GUI poll 门为第一公民、无回调 API；tell 无同步反馈、用户操作走
      ask。验证：文档 review 对照 design D6/D8/D9 逐条核对。
