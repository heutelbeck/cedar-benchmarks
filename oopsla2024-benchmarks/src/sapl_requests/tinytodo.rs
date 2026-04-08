use cedar_policy_core::ast::Request;
use smol_str::SmolStr;

use crate::entity_graph::EntityGraph;
use crate::utils;

use super::{
    build_entity_graph, default_algorithm, sapl_action, sapl_uid, sapl_uid_from_pv, SaplBatch,
    SaplSubscription,
};

/// Build a SAPL batch for the TinyTodo scenario from Cedar entities and requests.
///
/// SAPL variables needed:
/// - `entityGraph`: { uid -> [parent_uids...] } for graph.transitiveClosureSet()
/// - `lists`: { list_uid -> { owner, readers, editors } }
pub fn build_batch(entities: &impl EntityGraph, requests: &[Request]) -> SaplBatch {
    let entity_graph = build_entity_graph(entities);
    let mut lists = serde_json::Map::new();

    for entity in entities.iter() {
        let uid = sapl_uid(&entity.uid());
        let ty = entity.uid().entity_type().to_string();
        let attrs: std::collections::HashMap<SmolStr, &cedar_policy_core::ast::PartialValue> =
            entity.attrs().map(|(k, v)| (k.clone(), v)).collect();

        if ty == "List" {
            let mut list = serde_json::Map::new();
            // owner is an EntityUID (User)
            if let Some(pv) = attrs.get("owner") {
                list.insert(
                    "owner".to_string(),
                    serde_json::Value::String(sapl_uid_from_pv(pv)),
                );
            }
            // readers is an EntityUID (Team)
            if let Some(pv) = attrs.get("readers") {
                list.insert(
                    "readers".to_string(),
                    serde_json::Value::String(sapl_uid_from_pv(pv)),
                );
            }
            // editors is an EntityUID (Team)
            if let Some(pv) = attrs.get("editors") {
                list.insert(
                    "editors".to_string(),
                    serde_json::Value::String(sapl_uid_from_pv(pv)),
                );
            }
            lists.insert(uid, serde_json::Value::Object(list));
        }
    }

    let variables = serde_json::json!({
        "entityGraph": entity_graph,
        "lists": lists
    });

    // TinyTodo actions include "CreateList", "GetLists", "GetList", "UpdateList",
    // "CreateTask", "UpdateTask", "DeleteTask". The resource for app-level actions
    // is "Application::TinyTodo" in SAPL format.
    let subscriptions = requests
        .iter()
        .map(|r| {
            let resource_uid = sapl_uid(r.resource().uid().unwrap());
            // Convert Application::"TinyTodo" to the SAPL-expected "Application::TinyTodo"
            SaplSubscription {
                subject: serde_json::Value::String(sapl_uid(r.principal().uid().unwrap())),
                action: serde_json::Value::String(sapl_action(r)),
                resource: serde_json::Value::String(resource_uid),
            }
        })
        .collect();

    SaplBatch {
        app: "tinytodo".to_string(),
        variables,
        algorithm: default_algorithm(),
        subscriptions,
    }
}
