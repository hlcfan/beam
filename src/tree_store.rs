use std::collections::HashMap;

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
        COLLECTION_ROOT_ORDER_FILE_NAME, CollectionManifestFile, ManifestNode, NodeKind,
        RootOrderFile,
    };

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
}
