use cedar_policy_core::ast::Request;
use smol_str::SmolStr;

use crate::entity_graph::EntityGraph;
use crate::utils;

use super::{
    build_entity_graph, default_algorithm, sapl_action, sapl_uid, sapl_uid_from_pv, SaplBatch,
    SaplSubscription,
};

/// Build a SAPL batch for the GDrive scenario from Cedar entities and requests.
///
/// SAPL variables needed:
/// - `entityGraph`: { uid -> [parent_uids...] } for graph.transitiveClosureSet()
/// - `users`: { user_uid -> { documentsAndFoldersWithViewAccess, ownedDocuments, ownedFolders } }
/// - `docs`: { doc_uid -> { isPublic } }
pub fn build_batch(entities: &impl EntityGraph, requests: &[Request]) -> SaplBatch {
    let entity_graph = build_entity_graph(entities);
    let mut users = serde_json::Map::new();
    let mut docs = serde_json::Map::new();

    for entity in entities.iter() {
        let uid = sapl_uid(&entity.uid());
        let ty = entity.uid().entity_type().to_string();
        let attrs: std::collections::HashMap<SmolStr, &cedar_policy_core::ast::PartialValue> =
            entity.attrs().map(|(k, v)| (k.clone(), v)).collect();

        match ty.as_str() {
            "User" => {
                let mut user = serde_json::Map::new();
                // documentsAndFoldersWithViewAccess is an EntityUID (View entity)
                if let Some(pv) = attrs.get("documentsAndFoldersWithViewAccess") {
                    user.insert(
                        "documentsAndFoldersWithViewAccess".to_string(),
                        serde_json::Value::String(sapl_uid_from_pv(pv)),
                    );
                }
                // ownedDocuments is a Set of Document EntityUIDs (always include, default empty)
                let owned_docs: Vec<serde_json::Value> = attrs.get("ownedDocuments")
                    .map(|pv| utils::pv_expect_set_euids(pv)
                        .map(|e| serde_json::Value::String(sapl_uid(&e)))
                        .collect())
                    .unwrap_or_default();
                user.insert("ownedDocuments".to_string(), serde_json::Value::Array(owned_docs));
                // ownedFolders is a Set of Folder EntityUIDs (always include, default empty)
                let owned_folders: Vec<serde_json::Value> = attrs.get("ownedFolders")
                    .map(|pv| utils::pv_expect_set_euids(pv)
                        .map(|e| serde_json::Value::String(sapl_uid(&e)))
                        .collect())
                    .unwrap_or_default();
                user.insert("ownedFolders".to_string(), serde_json::Value::Array(owned_folders));
                users.insert(uid, serde_json::Value::Object(user));
            }
            "Document" => {
                let is_public = attrs.get("isPublic")
                    .map(|pv| utils::pv_expect_bool(pv))
                    .unwrap_or(false);
                let mut doc = serde_json::Map::new();
                doc.insert("isPublic".to_string(), serde_json::Value::Bool(is_public));
                docs.insert(uid, serde_json::Value::Object(doc));
            }
            _ => {} // Folders, Groups, Views - no per-entity variable data needed
        }
    }

    let variables = serde_json::json!({
        "entityGraph": entity_graph,
        "users": users,
        "docs": docs
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
        app: "gdrive".to_string(),
        variables,
        algorithm: default_algorithm(),
        subscriptions,
    }
}
