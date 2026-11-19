//! Catalog：服务条目表（design D8）——启动静态扫描 + 显式 reload，无 watch。

use service_core::{ServiceFactory, ServiceName};
use std::path::PathBuf;

/// 条目元数据（v1：描述；abi 版本经 DLL 指纹/符号在 load 时校验）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServiceMeta {
    pub description: Option<String>,
}

/// 符号来源——内部服务与 DLL 服务的**唯一区别**（D8：load 流程按 origin 分支）。
#[derive(Clone, Debug)]
pub enum ServiceOrigin {
    /// 内部 crate 服务：编译期工厂（启动时显式 register）。
    Internal(ServiceFactory),
    /// 外围 DLL：扫描目录得到的产物路径，load 时 dlopen 取符号。
    Dll(PathBuf),
}

// 手写: derive 会对裸 fn 指针做 `==` (unpredictable_function_pointer_comparisons);
// `fn_addr_eq` 是官方认可的比较方式 (同编译单元内函数地址稳定)。
impl PartialEq for ServiceOrigin {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (ServiceOrigin::Internal(a), ServiceOrigin::Internal(b)) => {
                std::ptr::fn_addr_eq(*a, *b)
            }
            (ServiceOrigin::Dll(a), ServiceOrigin::Dll(b)) => a == b,
            _ => false,
        }
    }
}
impl Eq for ServiceOrigin {}

/// Catalog 条目。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceDescriptor {
    pub name: ServiceName,
    pub origin: ServiceOrigin,
    pub meta: ServiceMeta,
}

impl ServiceDescriptor {
    pub fn internal(name: impl Into<ServiceName>, factory: ServiceFactory) -> Self {
        ServiceDescriptor {
            name: name.into(),
            origin: ServiceOrigin::Internal(factory),
            meta: ServiceMeta::default(),
        }
    }

    pub fn dll(name: impl Into<ServiceName>, path: PathBuf) -> Self {
        ServiceDescriptor {
            name: name.into(),
            origin: ServiceOrigin::Dll(path),
            meta: ServiceMeta::default(),
        }
    }
}

/// 服务条目表：同名**新条目替换旧条目**（reload 合并规则的最小方案）。
#[derive(Debug, Default)]
pub struct Catalog {
    entries: std::collections::HashMap<ServiceName, ServiceDescriptor>,
}

impl Catalog {
    pub fn new() -> Self {
        Self::default()
    }

    /// 插入/替换同名条目，返回被替换的旧条目。
    pub fn insert(&mut self, desc: ServiceDescriptor) -> Option<ServiceDescriptor> {
        self.entries.insert(desc.name.clone(), desc)
    }

    pub fn get(&self, name: &ServiceName) -> Option<&ServiceDescriptor> {
        self.entries.get(name)
    }

    pub fn contains(&self, name: &ServiceName) -> bool {
        self.entries.contains_key(name)
    }

    pub fn remove(&mut self, name: &ServiceName) -> Option<ServiceDescriptor> {
        self.entries.remove(name)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &ServiceDescriptor> {
        self.entries.values()
    }

    pub fn names(&self) -> Vec<ServiceName> {
        let mut names: Vec<_> = self.entries.keys().cloned().collect();
        names.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        names
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_service(_ctx: service_core::ServiceCtx) -> Box<dyn service_core::Service> {
        struct A;
        impl service_core::Service for A {
            fn start(&mut self) -> Result<(), String> {
                Ok(())
            }
            fn handle(
                &mut self,
                _env: service_core::Envelope,
            ) -> service_core::ServiceFuture<'_> {
                Box::pin(std::future::ready(()))
            }
        }
        Box::new(A)
    }

    #[test]
    fn insert_replaces_same_name() {
        let mut cat = Catalog::new();
        let first = ServiceDescriptor::dll("demo", PathBuf::from("/old/demo.dylib"));
        let second = ServiceDescriptor::dll("demo", PathBuf::from("/new/demo.dylib"));
        assert!(cat.insert(first).is_none());
        let old = cat.insert(second).expect("同名替换应返回旧条目");
        assert_eq!(old.origin, ServiceOrigin::Dll(PathBuf::from("/old/demo.dylib")));
        assert_eq!(cat.len(), 1, "替换不增条目数");
        match &cat.get(&"demo".into()).unwrap().origin {
            ServiceOrigin::Dll(p) => assert_eq!(p, &PathBuf::from("/new/demo.dylib")),
            other => panic!("origin 不对: {other:?}"),
        }
    }

    #[test]
    fn internal_descriptor_carries_factory() {
        struct Noop;
        impl service_core::Messaging for Noop {
            fn instance_id(&self) -> service_core::InstanceId {
                service_core::InstanceId(0)
            }
            fn self_address(&self) -> service_core::Address {
                service_core::Address::instance(service_core::InstanceId(0))
            }
            fn tell(&self, _env: service_core::Envelope) -> Result<(), service_core::RouteError> {
                Ok(())
            }
            fn call(
                &self,
                _env: service_core::Envelope,
            ) -> service_core::CallFuture {
                Box::pin(std::future::ready(Err(service_core::RouteError::NoSuchRoute)))
            }
            fn reply(
                &self,
                _req: &service_core::Envelope,
                _kind: &'static str,
                _payload: service_core::Payload,
            ) -> Result<(), service_core::RouteError> {
                Ok(())
            }

            fn reject(
                &self,
                _env: &service_core::Envelope,
            ) -> Result<(), service_core::RouteError> {
                Ok(())
            }

            fn spawn_task(&self, _fut: service_core::TaskFuture) {}
        }
        let ctx = service_core::ServiceCtx {
            instance: service_core::InstanceId(0),
            service: "fake".into(),
            messaging: std::sync::Arc::new(Noop),
            runtime_token: std::sync::Arc::new(NullToken),
        };

        struct NullToken;
        impl service_core::RuntimeToken for NullToken {
            fn handle_ptr(&self) -> *const () {
                std::ptr::null()
            }
        }
        let desc = ServiceDescriptor::internal("fake", a_service as ServiceFactory);
        assert_eq!(desc.name.as_str(), "fake");
        match desc.origin {
            ServiceOrigin::Internal(f) => {
                let mut svc = f(ctx);
                assert!(svc.start().is_ok());
            }
            other => panic!("origin 不对: {other:?}"),
        }
    }

    #[test]
    fn remove_and_lookup() {
        let mut cat = Catalog::new();
        cat.insert(ServiceDescriptor::internal("x", a_service as ServiceFactory));
        assert!(cat.contains(&"x".into()));
        assert!(cat.remove(&"x".into()).is_some());
        assert!(cat.get(&"x".into()).is_none());
        assert!(cat.is_empty());
    }
}
