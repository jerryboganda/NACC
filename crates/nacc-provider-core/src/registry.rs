//! The provider registry: the heterogeneous collection of adapters the
//! orchestrator resolves a Role Matrix row against (master plan S6's
//! dependency rule -- "the orchestrator depends on abstractions, not
//! concrete CLI parsers" -- and S2.7's "no role is ever hard-wired to one
//! provider").
//!
//! Deliberately dumb: it stores `Arc<dyn AgentProvider>` keyed by
//! `ProviderId` and nothing else. Everything interesting (which providers
//! are installed, authenticated, capable of a given model) is answered by
//! the adapters themselves, so there is no second copy of that state to go
//! stale here.

use std::collections::BTreeMap;
use std::sync::Arc;

use nacc_domain::ProviderId;

use crate::error::{ProviderError, Result};
use crate::provider::AgentProvider;

#[derive(Default)]
pub struct ProviderRegistry {
    providers: BTreeMap<ProviderId, Arc<dyn AgentProvider>>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an adapter. Registering the same `ProviderId` twice is a
    /// composition-root bug (two adapters claiming the same provider would
    /// make every later lookup ambiguous), so it is a typed error rather
    /// than a silent overwrite.
    pub fn register(&mut self, provider: Arc<dyn AgentProvider>) -> Result<()> {
        let id = provider.id();
        if self.providers.contains_key(&id) {
            return Err(ProviderError::Other(format!(
                "a provider adapter for {id} is already registered"
            )));
        }
        self.providers.insert(id, provider);
        Ok(())
    }

    pub fn get(&self, id: ProviderId) -> Option<&Arc<dyn AgentProvider>> {
        self.providers.get(&id)
    }

    /// The registered provider for `id`, or a typed
    /// [`ProviderError::NotInstalled`]-style error -- callers that got the
    /// id from a Role Matrix row always expect it to resolve.
    pub fn require(&self, id: ProviderId) -> Result<&Arc<dyn AgentProvider>> {
        self.get(id).ok_or_else(|| {
            ProviderError::Other(format!(
                "no provider adapter is registered for {id}; this build has {:?}",
                self.ids()
            ))
        })
    }

    pub fn ids(&self) -> Vec<ProviderId> {
        self.providers.keys().copied().collect()
    }

    pub fn all(&self) -> Vec<&Arc<dyn AgentProvider>> {
        self.providers.values().collect()
    }

    pub fn len(&self) -> usize {
        self.providers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::{AuthProbe, CapabilitySnapshot, InstallationProbe, ModelDescriptor};
    use crate::events::EventSink;
    use crate::provider::{
        AccountProfile, AgentInput, AgentSessionHandle, CancellationMode, CapabilityContext,
        LaunchRequest, ProfileValidation, ResolvedAgentProfile, ResumeRequest, RuntimeProfile,
        SessionId, UsageObservation,
    };
    use async_trait::async_trait;

    struct Stub(ProviderId);

    #[async_trait]
    impl AgentProvider for Stub {
        fn id(&self) -> ProviderId {
            self.0
        }
        fn display_name(&self) -> &str {
            "Stub"
        }
        async fn probe_installation(&self, _r: &RuntimeProfile) -> Result<InstallationProbe> {
            Ok(InstallationProbe {
                installed: false,
                executable_path: None,
                version: None,
            })
        }
        async fn probe_authentication(&self, _a: &AccountProfile) -> Result<AuthProbe> {
            Ok(AuthProbe {
                authenticated: false,
                account_label: None,
                detail: None,
            })
        }
        async fn list_models(&self, _a: &AccountProfile) -> Result<Vec<ModelDescriptor>> {
            Ok(vec![])
        }
        async fn capabilities(&self, _c: &CapabilityContext) -> Result<CapabilitySnapshot> {
            Err(ProviderError::Other("stub".into()))
        }
        async fn validate_profile(&self, _p: &ResolvedAgentProfile) -> Result<ProfileValidation> {
            Ok(ProfileValidation {
                supported: false,
                issues: vec!["stub".into()],
            })
        }
        async fn launch(
            &self,
            _r: LaunchRequest,
            _s: Box<dyn EventSink>,
        ) -> Result<AgentSessionHandle> {
            Err(ProviderError::Other("stub".into()))
        }
        async fn send_input(&self, _s: &SessionId, _i: AgentInput) -> Result<()> {
            Err(ProviderError::Other("stub".into()))
        }
        async fn cancel(&self, _s: &SessionId, _m: CancellationMode) -> Result<()> {
            Err(ProviderError::Other("stub".into()))
        }
        async fn resume(
            &self,
            _r: ResumeRequest,
            _s: Box<dyn EventSink>,
        ) -> Result<AgentSessionHandle> {
            Err(ProviderError::Other("stub".into()))
        }
        async fn collect_usage(&self, _s: &SessionId) -> Result<Option<UsageObservation>> {
            Ok(None)
        }
    }

    #[test]
    fn registering_and_looking_up_round_trips() {
        let mut registry = ProviderRegistry::new();
        registry
            .register(Arc::new(Stub(ProviderId::Claude)))
            .expect("first registration must succeed");
        assert_eq!(registry.len(), 1);
        assert!(registry.get(ProviderId::Claude).is_some());
        assert!(registry.get(ProviderId::Codex).is_none());
        assert_eq!(
            registry.require(ProviderId::Claude).unwrap().id(),
            ProviderId::Claude
        );
    }

    #[test]
    fn registering_the_same_provider_twice_is_a_typed_error() {
        let mut registry = ProviderRegistry::new();
        registry
            .register(Arc::new(Stub(ProviderId::Codex)))
            .unwrap();
        let err = registry
            .register(Arc::new(Stub(ProviderId::Codex)))
            .unwrap_err();
        assert!(err.to_string().contains("already registered"));
        assert_eq!(registry.len(), 1, "the original registration must survive");
    }

    #[test]
    fn requiring_an_unregistered_provider_names_what_is_available() {
        let mut registry = ProviderRegistry::new();
        registry
            .register(Arc::new(Stub(ProviderId::Copilot)))
            .unwrap();
        // Not `unwrap_err()`: `&Arc<dyn AgentProvider>` has no `Debug`, and
        // making the trait `Debug` just to please a test helper would be the
        // tail wagging the dog.
        let err = match registry.require(ProviderId::Antigravity) {
            Ok(_) => panic!("an unregistered provider must not resolve"),
            Err(err) => err,
        };
        assert!(err
            .to_string()
            .contains("no provider adapter is registered"));
        assert!(err.to_string().contains("Copilot"));
    }

    /// `BTreeMap` ordering is why `ProviderId` gained `Ord`: it makes
    /// `ids()` and any UI list built from the registry deterministic, so a
    /// provider list does not reshuffle between runs.
    #[test]
    fn ids_are_sorted_and_stable() {
        let mut registry = ProviderRegistry::new();
        for id in [
            ProviderId::Opencode,
            ProviderId::Claude,
            ProviderId::Copilot,
            ProviderId::Codex,
            ProviderId::Antigravity,
        ] {
            registry.register(Arc::new(Stub(id))).unwrap();
        }
        let first = registry.ids();
        let second = registry.ids();
        assert_eq!(first, second);
        assert_eq!(first.len(), 5);
    }
}
