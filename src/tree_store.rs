use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::models::RequestFile;
use crate::schema::SCHEMA_VERSION_V1;

pub const COLLECTION_MANIFEST_FILE_NAME: &str = ".manifest.toml";
pub const COLLECTION_ROOT_ORDER_FILE_NAME: &str = ".root-order.toml";

pub type NodeId = Ulid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Collection,
    Folder,
    Request,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub id: NodeId,
    pub name: String,
    pub kind: NodeKind,
    pub parent_id: Option<NodeId>,
    pub children: Vec<NodeId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SharedStore {
    pub nodes: HashMap<NodeId, Node>,
    // RequestFile stays the canonical request payload; the tree manifest only owns hierarchy.
    pub requests: HashMap<NodeId, RequestFile>,
    pub root_ids: Vec<NodeId>,
    // Uniqueness is enforced per parent scope with "root/<slug>" or "<parent_id>/<slug>" keys.
    pub name_index: HashMap<String, NodeId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameValidationError {
    EmptyName,
    DuplicateName { existing_id: NodeId },
}

impl SharedStore {
    pub fn rebuild_name_index(&mut self) -> Vec<String> {
        let mut next_index = HashMap::with_capacity(self.nodes.len());
        let mut warnings = Vec::new();

        for node in self.nodes.values() {
            let key = scope_key(node.parent_id, &node.name);
            if let Some(existing_id) = next_index.insert(key.clone(), node.id) {
                warnings.push(format!(
                    "Duplicate scoped name key `{key}` for nodes {existing_id} and {}.",
                    node.id
                ));
            }
        }

        self.name_index = next_index;
        warnings
    }
}

pub fn scope_key(parent_id: Option<NodeId>, name: &str) -> String {
    let scope_prefix = parent_id
        .map(|id| id.to_string())
        .unwrap_or_else(|| "root".to_string());
    format!("{scope_prefix}/{}", slugify_name(name))
}

pub fn assert_name_unique(
    name_index: &HashMap<String, NodeId>,
    parent_id: Option<NodeId>,
    name: &str,
    skip_id: Option<NodeId>,
) -> Result<(), NameValidationError> {
    if name.trim().is_empty() {
        return Err(NameValidationError::EmptyName);
    }

    let key = scope_key(parent_id, name);
    match name_index.get(&key).copied() {
        Some(existing_id) if Some(existing_id) != skip_id => {
            Err(NameValidationError::DuplicateName { existing_id })
        }
        _ => Ok(()),
    }
}

pub fn find_unique_name(
    name_index: &HashMap<String, NodeId>,
    parent_id: Option<NodeId>,
    preferred_name: &str,
    skip_id: Option<NodeId>,
) -> String {
    let base_name = normalize_name(preferred_name);
    if assert_name_unique(name_index, parent_id, &base_name, skip_id).is_ok() {
        return base_name;
    }

    for suffix in 2.. {
        let candidate = format!("{base_name} {suffix}");
        if assert_name_unique(name_index, parent_id, &candidate, skip_id).is_ok() {
            return candidate;
        }
    }

    unreachable!("infinite numeric suffix space must eventually produce a unique name")
}

pub fn root_collection_id_of(store: &SharedStore, node_id: NodeId) -> Option<NodeId> {
    let mut cursor = node_id;
    let mut seen = HashSet::new();

    loop {
        let node = store.nodes.get(&cursor)?;
        if node.kind == NodeKind::Collection {
            return Some(node.id);
        }

        let parent_id = node.parent_id?;
        if !seen.insert(parent_id) {
            return None;
        }
        cursor = parent_id;
    }
}

fn normalize_name(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        "Untitled".to_string()
    } else {
        trimmed.to_string()
    }
}

fn slugify_name(input: &str) -> String {
    let normalized = normalize_name(input);
    let mut out = String::with_capacity(normalized.len());
    let mut prev_dash = false;
    for ch in normalized.chars() {
        let lowered = ch.to_ascii_lowercase();
        if lowered.is_ascii_alphanumeric() {
            out.push(lowered);
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    let normalized = out.trim_matches('-').to_string();
    if normalized.is_empty() {
        "untitled".to_string()
    } else {
        normalized
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootOrderFile {
    pub schema_version: u32,
    #[serde(default)]
    pub root_ids: Vec<NodeId>,
}

impl Default for RootOrderFile {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION_V1,
            root_ids: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectionManifestFile {
    pub schema_version: u32,
    pub id: NodeId,
    pub name: String,
    pub kind: NodeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<ManifestNode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestNode {
    pub id: NodeId,
    pub name: String,
    pub kind: NodeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<ManifestNode>,
}

#[cfg(test)]
mod tests {
    use super::{
        COLLECTION_ROOT_ORDER_FILE_NAME, CollectionManifestFile, ManifestNode, NameValidationError,
        Node, NodeKind, RootOrderFile, SharedStore, assert_name_unique, find_unique_name,
        root_collection_id_of, scope_key,
    };
    use std::collections::HashMap;

    #[test]
    fn root_order_file_defaults_to_current_schema_version() {
        let file = RootOrderFile::default();
        assert_eq!(file.schema_version, crate::schema::SCHEMA_VERSION_V1);
        assert!(file.root_ids.is_empty());
        assert_eq!(COLLECTION_ROOT_ORDER_FILE_NAME, ".root-order.toml");
    }

    #[test]
    fn collection_manifest_roundtrips_nested_children_shape() {
        let manifest = CollectionManifestFile {
            schema_version: crate::schema::SCHEMA_VERSION_V1,
            id: ulid::Ulid::new(),
            name: "Users".to_string(),
            kind: NodeKind::Collection,
            description: Some("Shared collection metadata lives in the manifest".to_string()),
            created_at: None,
            updated_at: None,
            children: vec![ManifestNode {
                id: ulid::Ulid::new(),
                name: "Admin".to_string(),
                kind: NodeKind::Folder,
                description: Some("Folder metadata also lives in the manifest".to_string()),
                created_at: None,
                updated_at: None,
                children: vec![ManifestNode {
                    id: ulid::Ulid::new(),
                    name: "Get User".to_string(),
                    kind: NodeKind::Request,
                    description: None,
                    created_at: None,
                    updated_at: None,
                    children: Vec::new(),
                }],
            }],
        };

        let encoded = toml::to_string_pretty(&manifest).expect("encode manifest");
        let decoded: CollectionManifestFile = toml::from_str(&encoded).expect("decode manifest");

        assert_eq!(decoded.kind, NodeKind::Collection);
        assert_eq!(decoded.children.len(), 1);
        assert_eq!(decoded.children[0].kind, NodeKind::Folder);
        assert_eq!(decoded.children[0].children.len(), 1);
        assert_eq!(decoded.children[0].children[0].kind, NodeKind::Request);
    }

    #[test]
    fn scope_key_uses_root_or_parent_scope_with_slugified_name() {
        let parent_id = ulid::Ulid::new();
        assert_eq!(scope_key(None, "My Collection"), "root/my-collection");
        assert_eq!(
            scope_key(Some(parent_id), "Fetch User"),
            format!("{parent_id}/fetch-user")
        );
    }

    #[test]
    fn name_uniqueness_checks_allow_same_node_and_reject_conflicts() {
        let parent_id = ulid::Ulid::new();
        let existing_id = ulid::Ulid::new();
        let index = HashMap::from([(scope_key(Some(parent_id), "Get User"), existing_id)]);

        assert_eq!(
            assert_name_unique(&index, Some(parent_id), "get-user", None),
            Err(NameValidationError::DuplicateName { existing_id })
        );
        assert_eq!(
            assert_name_unique(&index, Some(parent_id), "Get User", Some(existing_id)),
            Ok(())
        );
        assert_eq!(
            assert_name_unique(&index, Some(parent_id), "   ", None),
            Err(NameValidationError::EmptyName)
        );
    }

    #[test]
    fn find_unique_name_increments_suffix_within_parent_scope() {
        let parent_id = ulid::Ulid::new();
        let index = HashMap::from([
            (scope_key(Some(parent_id), "Request"), ulid::Ulid::new()),
            (scope_key(Some(parent_id), "Request 2"), ulid::Ulid::new()),
            (scope_key(None, "Request"), ulid::Ulid::new()),
        ]);

        assert_eq!(
            find_unique_name(&index, Some(parent_id), "Request", None),
            "Request 3"
        );
        assert_eq!(
            find_unique_name(&index, Some(parent_id), "Fresh Name", None),
            "Fresh Name"
        );
        assert_eq!(
            find_unique_name(&index, Some(parent_id), "   ", None),
            "Untitled"
        );
    }

    #[test]
    fn root_collection_lookup_walks_to_collection_ancestor() {
        let collection_id = ulid::Ulid::new();
        let folder_id = ulid::Ulid::new();
        let request_id = ulid::Ulid::new();
        let store = SharedStore {
            nodes: HashMap::from([
                (
                    collection_id,
                    Node {
                        id: collection_id,
                        name: "API".to_string(),
                        kind: NodeKind::Collection,
                        parent_id: None,
                        children: vec![folder_id],
                    },
                ),
                (
                    folder_id,
                    Node {
                        id: folder_id,
                        name: "Users".to_string(),
                        kind: NodeKind::Folder,
                        parent_id: Some(collection_id),
                        children: vec![request_id],
                    },
                ),
                (
                    request_id,
                    Node {
                        id: request_id,
                        name: "Get User".to_string(),
                        kind: NodeKind::Request,
                        parent_id: Some(folder_id),
                        children: Vec::new(),
                    },
                ),
            ]),
            requests: HashMap::new(),
            root_ids: vec![collection_id],
            name_index: HashMap::new(),
        };

        assert_eq!(
            root_collection_id_of(&store, request_id),
            Some(collection_id)
        );
        assert_eq!(
            root_collection_id_of(&store, folder_id),
            Some(collection_id)
        );
        assert_eq!(
            root_collection_id_of(&store, collection_id),
            Some(collection_id)
        );
    }

    #[test]
    fn rebuild_name_index_collects_duplicate_scope_warnings() {
        let parent_id = ulid::Ulid::new();
        let first_id = ulid::Ulid::new();
        let second_id = ulid::Ulid::new();
        let mut store = SharedStore {
            nodes: HashMap::from([
                (
                    first_id,
                    Node {
                        id: first_id,
                        name: "Get User".to_string(),
                        kind: NodeKind::Request,
                        parent_id: Some(parent_id),
                        children: Vec::new(),
                    },
                ),
                (
                    second_id,
                    Node {
                        id: second_id,
                        name: "get-user".to_string(),
                        kind: NodeKind::Request,
                        parent_id: Some(parent_id),
                        children: Vec::new(),
                    },
                ),
            ]),
            requests: HashMap::new(),
            root_ids: Vec::new(),
            name_index: HashMap::new(),
        };

        let warnings = store.rebuild_name_index();
        assert_eq!(warnings.len(), 1);
        assert_eq!(store.name_index.len(), 1);
    }
}
