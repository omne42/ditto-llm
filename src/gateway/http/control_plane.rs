use std::collections::{BTreeMap, HashMap};

use crate::gateway::router::Router as GatewayRouter;
use crate::gateway::{Gateway, GatewayError, RouterConfig, VirtualKeyConfig};

use super::GatewayHttpState;

#[derive(Clone, Debug)]
pub(super) struct GatewayControlPlaneSnapshot {
    pub(super) virtual_keys: Vec<VirtualKeyConfig>,
    virtual_key_token_index: HashMap<String, usize>,
    pub(super) router_config: RouterConfig,
    pub(super) router: GatewayRouter,
    pub(super) backend_names: Vec<String>,
    pub(super) backend_model_maps: HashMap<String, BTreeMap<String, String>>,
}

impl GatewayControlPlaneSnapshot {
    pub(super) fn from_gateway(gateway: &Gateway) -> Self {
        let virtual_keys = gateway.list_virtual_keys();
        let mut virtual_key_token_index = HashMap::new();
        for (idx, key) in virtual_keys.iter().enumerate() {
            virtual_key_token_index
                .entry(key.token.clone())
                .or_insert(idx);
        }

        let router_config = gateway.router_config();
        let router = GatewayRouter::new(router_config.clone());
        let backend_names = gateway.backend_names();
        let backend_model_maps = gateway.backend_model_maps();

        Self {
            virtual_keys,
            virtual_key_token_index,
            router_config,
            router,
            backend_names,
            backend_model_maps,
        }
    }

    fn uses_virtual_keys(&self) -> bool {
        !self.virtual_keys.is_empty()
    }

    fn virtual_key_by_token(&self, token: &str) -> Option<&VirtualKeyConfig> {
        if let Some(index) = self.virtual_key_token_index.get(token).copied()
            && let Some(key) = self.virtual_keys.get(index)
            && key.token == token
        {
            return Some(key);
        }
        self.virtual_keys.iter().find(|key| key.token == token)
    }
}

impl GatewayHttpState {
    fn with_control_plane<R>(&self, f: impl FnOnce(&GatewayControlPlaneSnapshot) -> R) -> R {
        let snapshot = self
            .control_plane
            .read()
            .unwrap_or_else(|poison| poison.into_inner());
        f(&snapshot)
    }

    fn replace_control_plane_snapshot(&self, snapshot: GatewayControlPlaneSnapshot) {
        let mut slot = self
            .control_plane
            .write()
            .unwrap_or_else(|poison| poison.into_inner());
        *slot = snapshot;
    }

    pub(crate) fn uses_virtual_keys(&self) -> bool {
        self.with_control_plane(GatewayControlPlaneSnapshot::uses_virtual_keys)
    }

    pub(crate) fn virtual_key_by_token(&self, token: &str) -> Option<VirtualKeyConfig> {
        self.with_control_plane(|snapshot| snapshot.virtual_key_by_token(token).cloned())
    }

    pub(crate) fn list_virtual_keys_snapshot(&self) -> Vec<VirtualKeyConfig> {
        self.with_control_plane(|snapshot| snapshot.virtual_keys.clone())
    }

    pub(crate) fn router_config_snapshot(&self) -> RouterConfig {
        self.with_control_plane(|snapshot| snapshot.router_config.clone())
    }

    pub(crate) fn backend_names_snapshot(&self) -> Vec<String> {
        self.with_control_plane(|snapshot| snapshot.backend_names.clone())
    }

    pub(crate) fn backend_model_map(&self, backend_name: &str) -> BTreeMap<String, String> {
        self.with_control_plane(|snapshot| {
            snapshot
                .backend_model_maps
                .get(backend_name)
                .cloned()
                .unwrap_or_default()
        })
    }

    pub(crate) fn guardrails_for_model(
        &self,
        model: Option<&str>,
        key: &VirtualKeyConfig,
    ) -> crate::gateway::GuardrailsConfig {
        self.with_control_plane(|snapshot| {
            model
                .and_then(|model_id| {
                    snapshot
                        .router
                        .rule_for_model(model_id, Some(key))
                        .and_then(|rule| rule.guardrails.as_ref())
                })
                .unwrap_or(&key.guardrails)
                .clone()
        })
    }

    pub(crate) fn select_backends_for_model_seeded(
        &self,
        model: &str,
        key: Option<&VirtualKeyConfig>,
        seed: Option<&str>,
    ) -> Result<Vec<String>, GatewayError> {
        self.with_control_plane(|snapshot| {
            snapshot
                .router
                .select_backends_for_model_seeded(model, key, seed)
        })
    }

    #[cfg(feature = "gateway-costing")]
    pub(crate) fn mapped_backend_model(
        &self,
        backend_name: &str,
        request_model: &str,
    ) -> Option<String> {
        self.with_control_plane(|snapshot| {
            snapshot
                .backend_model_maps
                .get(backend_name)
                .and_then(|model_map| {
                    model_map
                        .get(request_model)
                        .or_else(|| model_map.get("*"))
                        .cloned()
                })
        })
    }

    pub(crate) fn sync_control_plane_from_gateway(&self) {
        let snapshot = GatewayControlPlaneSnapshot::from_gateway(&self.gateway);
        self.replace_control_plane_snapshot(snapshot);
    }
}
