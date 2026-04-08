pub mod github;
pub mod gdrive;
pub mod tinytodo;

use cedar_policy_core::ast::{EntityUID, PartialValue, Request};
use serde::Serialize;

use crate::entity_graph::EntityGraph;

/// Convert a Cedar EntityUID to the SAPL-style string format.
/// Cedar: `User::"user_0"` -> SAPL: `User::user_0` (no inner quotes).
/// No normalization - preserves exact Cedar UIDs to maintain entity identity.
pub fn sapl_uid(euid: &EntityUID) -> String {
    let ty = euid.entity_type().to_string();
    let eid = euid.eid().to_string();
    format!("{ty}::{eid}")
}

/// Convert a Cedar EntityUID to a SAPL-style string from a PartialValue.
pub fn sapl_uid_from_pv(pv: &PartialValue) -> String {
    sapl_uid(&crate::utils::pv_expect_euid(pv))
}

/// Extract bare action name from a Cedar action EntityUID.
/// Cedar: `Action::"read"` -> SAPL: `"read"`.
pub fn sapl_action(request: &Request) -> String {
    request.action().uid().unwrap().eid().to_string()
}

/// Build the entity graph variable from raw entity JSON.
/// Uses EntityJson serialization to get the original direct parents,
/// avoiding any transitive closure that entity.ancestors() might return.
pub fn build_entity_graph(entities: &impl EntityGraph) -> serde_json::Value {
    let mut graph = serde_json::Map::new();
    for entity in entities.iter() {
        let uid = sapl_uid(&entity.uid());
        // Serialize entity to JSON to get the raw parents array
        let entity_json = cedar_policy_core::entities::EntityJson::from_entity(&entity).unwrap();
        let json_str = serde_json::to_string(&entity_json).unwrap();
        let json_val: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        let parents: Vec<serde_json::Value> = json_val
            .get("parents")
            .and_then(|p| p.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|parent| {
                        // Each parent is {"type": "Foo", "id": "bar"}
                        let ty = parent.get("type")?.as_str()?;
                        let id = parent.get("id")?.as_str()?;
                        Some(serde_json::Value::String(format!("{ty}::{id}")))
                    })
                    .collect()
            })
            .unwrap_or_default();
        graph.insert(uid, serde_json::Value::Array(parents));
    }
    serde_json::Value::Object(graph)
}

/// A SAPL subscription matching AuthorizationSubscription JSON format.
#[derive(Debug, Serialize)]
pub struct SaplSubscription {
    pub subject: serde_json::Value,
    pub action: serde_json::Value,
    pub resource: serde_json::Value,
}

/// Build a SAPL batch from entity data and requests.
#[derive(Debug, Serialize)]
pub struct SaplBatch {
    pub app: String,
    pub variables: serde_json::Value,
    pub algorithm: serde_json::Value,
    pub subscriptions: Vec<SaplSubscription>,
}

/// Default SAPL algorithm (matches Cedar's default-deny behavior).
pub fn default_algorithm() -> serde_json::Value {
    serde_json::json!({
        "votingMode": "PRIORITY_DENY",
        "defaultDecision": "DENY",
        "errorHandling": "PROPAGATE"
    })
}
