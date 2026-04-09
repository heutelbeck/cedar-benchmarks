use cedar_policy_core::ast::Request;
use smol_str::SmolStr;

use crate::entity_graph::EntityGraph;
use crate::utils;

use super::{
    build_entity_graph, default_algorithm, sapl_action, sapl_uid, sapl_uid_from_pv, SaplBatch,
    SaplSubscription,
};

/// Build a SAPL batch for the GitHub scenario from Cedar entities and requests.
///
/// SAPL variables needed:
/// - `entityGraph`: { uid -> [parent_uids...] } for graph.transitiveClosureSet()
/// - `repos`: { repo_uid -> { readers, triagers, writers, maintainers, admins, owner } }
/// - `orgs`: { org_uid -> { readers, writers, admins } }
pub fn build_batch(entities: &impl EntityGraph, requests: &[Request]) -> SaplBatch {
    let entity_graph = build_entity_graph(entities);
    let mut repos = serde_json::Map::new();
    let mut orgs = serde_json::Map::new();

    for entity in entities.iter() {
        let uid = sapl_uid(&entity.uid());
        let ty = entity.uid().entity_type().to_string();
        let attrs: std::collections::HashMap<SmolStr, &cedar_policy_core::ast::PartialValue> =
            entity.attrs().map(|(k, v)| (k.clone(), v)).collect();

        match ty.as_str() {
            "Repository" => {
                let mut repo = serde_json::Map::new();
                for key in &["readers", "triagers", "writers", "maintainers", "admins"] {
                    if let Some(pv) = attrs.get(*key) {
                        repo.insert(key.to_string(), serde_json::Value::String(sapl_uid_from_pv(pv)));
                    }
                }
                if let Some(pv) = attrs.get("owner") {
                    repo.insert(
                        "owner".to_string(),
                        serde_json::Value::String(sapl_uid_from_pv(pv)),
                    );
                }
                repos.insert(uid, serde_json::Value::Object(repo));
            }
            "Organization" => {
                let mut org = serde_json::Map::new();
                for key in &["readers", "writers", "admins"] {
                    if let Some(pv) = attrs.get(*key) {
                        org.insert(key.to_string(), serde_json::Value::String(sapl_uid_from_pv(pv)));
                    }
                }
                orgs.insert(uid, serde_json::Value::Object(org));
            }
            _ => {} // Users, Teams, Permissions - no per-entity variable data needed
        }
    }

    let variables = serde_json::json!({
        "entityGraph": entity_graph,
        "repos": repos,
        "orgs": orgs
    });

    let subscriptions = requests
        .iter()
        .map(|r| SaplSubscription {
            subject: serde_json::Value::String(sapl_uid(r.principal().uid().unwrap())),
            action: serde_json::Value::String(sapl_action(r)),
            resource: serde_json::Value::String(sapl_uid(r.resource().uid().unwrap())),
        })
        .collect();

    SaplBatch {
        app: "github".to_string(),
        variables,
        algorithm: default_algorithm(),
        subscriptions,
    }
}
