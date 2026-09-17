//! Capability registry: id → provider implementations.

use crate::descriptor::CapabilityDescriptor;
use crate::error::CapabilityError;
use crate::host::CapabilityHost;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

/// A capability implementation. Mutating capabilities call
/// `host.run_command(...)` so effects stay transactional and undoable.
pub trait Capability: Send + Sync {
    fn descriptor(&self) -> CapabilityDescriptor;
    fn execute(
        &self,
        host: &mut dyn CapabilityHost,
        input: &Value,
    ) -> Result<Value, CapabilityError>;
}

#[derive(Default)]
pub struct CapabilityRegistry {
    /// capability id → providers (first registered is the default).
    providers: HashMap<String, Vec<Arc<dyn Capability>>>,
}

impl CapabilityRegistry {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn register(&mut self, cap: Arc<dyn Capability>) {
        self.providers
            .entry(cap.descriptor().id.clone())
            .or_default()
            .push(cap);
    }
    /// Default provider for a capability id.
    pub fn get(&self, id: &str) -> Option<Arc<dyn Capability>> {
        self.providers.get(id).and_then(|v| v.first()).cloned()
    }
    /// Specific provider for a capability id.
    pub fn get_provider(&self, id: &str, provider: &str) -> Option<Arc<dyn Capability>> {
        self.providers.get(id).and_then(|v| {
            v.iter().find(|c| c.descriptor().provider.id == provider).cloned()
        })
    }
    pub fn descriptors(&self) -> Vec<CapabilityDescriptor> {
        let mut out = Vec::new();
        for caps in self.providers.values() {
            for c in caps {
                out.push(c.descriptor());
            }
        }
        out.sort_by(|a, b| a.id.cmp(&b.id).then(a.provider.id.cmp(&b.provider.id)));
        out
    }
    pub fn contains(&self, id: &str) -> bool {
        self.providers.contains_key(id)
    }
}
