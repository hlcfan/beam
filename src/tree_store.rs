use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::error::{BeamError, Result};
use crate::models::{EnvironmentFile, RequestFile};
use crate::paths::BeamPaths;
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
    pub description: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
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
    // Environment files keyed by environment_id for O(1) lookups.
    pub environments: HashMap<NodeId, EnvironmentFile>,
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
) -> std::result::Result<(), NameValidationError> {
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

pub fn collection_dir_name(name: &str) -> String {
    slugify_name(name)
}

pub fn folder_dir_name(name: &str) -> String {
    slugify_name(name)
}

pub fn request_file_name(name: &str) -> String {
    format!("{}.request.toml", slugify_name(name))
}

pub fn collection_dir_path(
    paths: &BeamPaths,
    store: &SharedStore,
    collection_id: NodeId,
) -> Result<PathBuf> {
    let collection = node_by_kind(store, collection_id, NodeKind::Collection)?;
    Ok(paths
        .collections_dir
        .join(collection_dir_name(&collection.name)))
}

pub fn folder_dir_path(
    paths: &BeamPaths,
    store: &SharedStore,
    folder_id: NodeId,
) -> Result<PathBuf> {
    let folder = node_by_kind(store, folder_id, NodeKind::Folder)?;
    let parent_id = folder.parent_id.ok_or_else(|| BeamError::Validation {
        message: format!("folder node {} is missing parent_id", folder_id),
    })?;
    Ok(node_dir_path(paths, store, parent_id)?.join(folder_dir_name(&folder.name)))
}

pub fn node_dir_path(paths: &BeamPaths, store: &SharedStore, node_id: NodeId) -> Result<PathBuf> {
    match node_by_id(store, node_id)?.kind {
        NodeKind::Collection => collection_dir_path(paths, store, node_id),
        NodeKind::Folder => folder_dir_path(paths, store, node_id),
        NodeKind::Request => Err(BeamError::Validation {
            message: format!("request node {} does not have a directory path", node_id),
        }),
    }
}

pub fn request_file_path(
    paths: &BeamPaths,
    store: &SharedStore,
    request_id: NodeId,
) -> Result<PathBuf> {
    let request = node_by_kind(store, request_id, NodeKind::Request)?;
    let parent_id = request.parent_id.ok_or_else(|| BeamError::Validation {
        message: format!("request node {} is missing parent_id", request_id),
    })?;
    let parent = node_by_id(store, parent_id)?;
    let request_dir = match parent.kind {
        NodeKind::Collection => collection_dir_path(paths, store, parent_id)?,
        NodeKind::Folder => node_dir_path(paths, store, parent_id)?,
        NodeKind::Request => {
            return Err(BeamError::Validation {
                message: format!(
                    "request node {} cannot be parent of request node {}",
                    parent_id, request_id
                ),
            });
        }
    };
    Ok(request_dir.join(request_file_name(&request.name)))
}

pub fn root_order_file(store: &SharedStore) -> RootOrderFile {
    RootOrderFile {
        schema_version: SCHEMA_VERSION_V1,
        root_ids: store.root_ids.clone(),
    }
}

pub fn collection_manifest_from_store(
    store: &SharedStore,
    collection_id: NodeId,
) -> Result<CollectionManifestFile> {
    let collection = node_by_kind(store, collection_id, NodeKind::Collection)?;
    let children = collection
        .children
        .iter()
        .copied()
        .map(|child_id| manifest_node_from_store(store, child_id))
        .collect::<Result<Vec<_>>>()?;

    Ok(CollectionManifestFile {
        schema_version: SCHEMA_VERSION_V1,
        id: collection.id,
        name: collection.name.clone(),
        kind: collection.kind,
        description: collection.description.clone(),
        created_at: collection.created_at,
        updated_at: collection.updated_at,
        children,
    })
}

pub fn shared_store_from_collection_manifest_path(manifest_path: &Path) -> Result<SharedStore> {
    let manifest: CollectionManifestFile = read_toml_file(manifest_path)?;
    if manifest.kind != NodeKind::Collection {
        return Err(BeamError::Validation {
            message: format!(
                "collection manifest {} declared kind {:?} instead of collection",
                manifest_path.display(),
                manifest.kind
            ),
        });
    }

    let mut store = SharedStore::default();
    let collection_id = manifest.id;
    insert_manifest_runtime_node(
        &mut store,
        Node {
            id: collection_id,
            name: manifest.name,
            kind: NodeKind::Collection,
            description: manifest.description,
            created_at: manifest.created_at,
            updated_at: manifest.updated_at,
            parent_id: None,
            children: Vec::new(),
        },
    )?;
    store.root_ids.push(collection_id);

    let child_ids = manifest
        .children
        .iter()
        .map(|child| load_manifest_child(&mut store, child, collection_id))
        .collect::<Result<Vec<_>>>()?;
    if let Some(collection) = store.nodes.get_mut(&collection_id) {
        collection.children = child_ids;
    }

    Ok(store)
}

pub fn write_collection_manifest(
    paths: &BeamPaths,
    store: &SharedStore,
    collection_id: NodeId,
) -> Result<PathBuf> {
    let collection_dir = collection_dir_path(paths, store, collection_id)?;
    fs::create_dir_all(&collection_dir).map_err(|source| BeamError::Io {
        path: collection_dir.clone(),
        source,
    })?;

    let manifest_path = collection_dir.join(COLLECTION_MANIFEST_FILE_NAME);
    let manifest = collection_manifest_from_store(store, collection_id)?;
    write_toml_file(&manifest_path, &manifest)?;
    Ok(manifest_path)
}

pub fn write_request_payload(
    paths: &BeamPaths,
    store: &SharedStore,
    request_id: NodeId,
) -> Result<PathBuf> {
    let request_path = request_file_path(paths, store, request_id)?;
    let request_dir = request_path
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| BeamError::Validation {
            message: format!(
                "request path {} has no parent directory",
                request_path.display()
            ),
        })?;
    fs::create_dir_all(&request_dir).map_err(|source| BeamError::Io {
        path: request_dir,
        source,
    })?;

    let mut request_file =
        store
            .requests
            .get(&request_id)
            .cloned()
            .ok_or_else(|| BeamError::NotFound {
                entity: "request_file",
                id: request_id.to_string(),
            })?;
    let request_node = node_by_kind(store, request_id, NodeKind::Request)?;
    if request_file.meta.request_id != request_id {
        return Err(BeamError::Validation {
            message: format!(
                "request payload {} declared request_id {}",
                request_id, request_file.meta.request_id
            ),
        });
    }

    request_file.meta.name = request_node.name.clone();
    request_file.file_path = Some(request_path.clone());
    write_toml_file(&request_path, &request_file)?;
    Ok(request_path)
}

pub fn write_root_order(paths: &BeamPaths, store: &SharedStore) -> Result<PathBuf> {
    fs::create_dir_all(&paths.collections_dir).map_err(|source| BeamError::Io {
        path: paths.collections_dir.clone(),
        source,
    })?;
    write_toml_file(&paths.collections_root_order_file, &root_order_file(store))?;
    Ok(paths.collections_root_order_file.clone())
}

pub fn persist_collection_subtree(
    paths: &BeamPaths,
    store: &SharedStore,
    collection_id: NodeId,
) -> Result<PathBuf> {
    let manifest_path = write_collection_manifest(paths, store, collection_id)?;
    write_request_payloads_in_subtree(paths, store, collection_id)?;
    Ok(manifest_path)
}

pub fn persist_shared_tree(paths: &BeamPaths, store: &SharedStore) -> Result<()> {
    for collection_id in store.root_ids.iter().copied() {
        persist_collection_subtree(paths, store, collection_id)?;
    }
    write_root_order(paths, store)?;
    Ok(())
}

fn manifest_node_from_store(store: &SharedStore, node_id: NodeId) -> Result<ManifestNode> {
    let node = node_by_id(store, node_id)?;
    if node.kind == NodeKind::Collection {
        return Err(BeamError::Validation {
            message: format!(
                "collection node {} cannot be nested under a collection manifest",
                node_id
            ),
        });
    }

    let children = node
        .children
        .iter()
        .copied()
        .map(|child_id| manifest_node_from_store(store, child_id))
        .collect::<Result<Vec<_>>>()?;

    if node.kind == NodeKind::Request && !children.is_empty() {
        return Err(BeamError::Validation {
            message: format!("request node {} cannot have child nodes", node_id),
        });
    }

    Ok(ManifestNode {
        id: node.id,
        name: node.name.clone(),
        kind: node.kind,
        description: node.description.clone(),
        created_at: node.created_at,
        updated_at: node.updated_at,
        children,
    })
}

fn write_request_payloads_in_subtree(
    paths: &BeamPaths,
    store: &SharedStore,
    node_id: NodeId,
) -> Result<()> {
    let node = node_by_id(store, node_id)?;
    match node.kind {
        NodeKind::Collection | NodeKind::Folder => {
            for child_id in node.children.iter().copied() {
                write_request_payloads_in_subtree(paths, store, child_id)?;
            }
        }
        NodeKind::Request => {
            write_request_payload(paths, store, node_id)?;
        }
    }
    Ok(())
}

fn load_manifest_child(
    store: &mut SharedStore,
    manifest_node: &ManifestNode,
    parent_id: NodeId,
) -> Result<NodeId> {
    if manifest_node.kind == NodeKind::Collection {
        return Err(BeamError::Validation {
            message: format!(
                "nested collection node {} cannot appear inside a collection manifest",
                manifest_node.id
            ),
        });
    }
    if manifest_node.kind == NodeKind::Request && !manifest_node.children.is_empty() {
        return Err(BeamError::Validation {
            message: format!("request node {} cannot have child nodes", manifest_node.id),
        });
    }

    let node_id = manifest_node.id;
    insert_manifest_runtime_node(
        store,
        Node {
            id: node_id,
            name: manifest_node.name.clone(),
            kind: manifest_node.kind,
            description: manifest_node.description.clone(),
            created_at: manifest_node.created_at,
            updated_at: manifest_node.updated_at,
            parent_id: Some(parent_id),
            children: Vec::new(),
        },
    )?;

    let child_ids = manifest_node
        .children
        .iter()
        .map(|child| load_manifest_child(store, child, node_id))
        .collect::<Result<Vec<_>>>()?;
    if let Some(node) = store.nodes.get_mut(&node_id) {
        node.children = child_ids;
    }

    Ok(node_id)
}

fn insert_manifest_runtime_node(store: &mut SharedStore, node: Node) -> Result<()> {
    if store.nodes.contains_key(&node.id) {
        return Err(BeamError::Validation {
            message: format!("duplicate node {} found while loading manifest", node.id),
        });
    }
    map_name_validation_error(assert_name_unique(
        &store.name_index,
        node.parent_id,
        &node.name,
        None,
    ))?;

    let key = scope_key(node.parent_id, &node.name);
    store.name_index.insert(key, node.id);
    store.nodes.insert(node.id, node);
    Ok(())
}

fn node_by_id(store: &SharedStore, node_id: NodeId) -> Result<&Node> {
    store
        .nodes
        .get(&node_id)
        .ok_or_else(|| BeamError::NotFound {
            entity: "node",
            id: node_id.to_string(),
        })
}

fn node_by_kind(store: &SharedStore, node_id: NodeId, expected_kind: NodeKind) -> Result<&Node> {
    let node = node_by_id(store, node_id)?;
    if node.kind != expected_kind {
        return Err(BeamError::Validation {
            message: format!(
                "node {} had kind {:?}, expected {:?}",
                node_id, node.kind, expected_kind
            ),
        });
    }
    Ok(node)
}

fn write_toml_file<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let encoded = toml::to_string_pretty(value)?;
    atomic_write(path, encoded.as_bytes())
}

fn read_toml_file<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let content = fs::read_to_string(path).map_err(|source| BeamError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    toml::from_str(&content).map_err(|source| BeamError::TomlDecode {
        path: path.to_path_buf(),
        source,
    })
}

fn map_name_validation_error(result: std::result::Result<(), NameValidationError>) -> Result<()> {
    match result {
        Ok(()) => Ok(()),
        Err(NameValidationError::EmptyName) => Err(BeamError::Validation {
            message: "name cannot be empty".to_string(),
        }),
        Err(NameValidationError::DuplicateName { existing_id }) => Err(BeamError::Validation {
            message: format!("duplicate name in scope conflicts with node {existing_id}"),
        }),
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp_path = temp_path(path);

    fs::write(&tmp_path, bytes).map_err(|source| BeamError::Io {
        path: tmp_path.clone(),
        source,
    })?;

    fs::rename(&tmp_path, path).map_err(|source| BeamError::Io {
        path: path.to_path_buf(),
        source,
    })?;

    Ok(())
}

fn temp_path(path: &Path) -> PathBuf {
    let mut file_name = path
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| "tmp".to_string());
    file_name.push_str(".tmp");
    path.with_file_name(file_name)
}

#[cfg(test)]
mod tests {
    use super::{
        COLLECTION_MANIFEST_FILE_NAME, COLLECTION_ROOT_ORDER_FILE_NAME, CollectionManifestFile,
        ManifestNode, NameValidationError, Node, NodeKind, RootOrderFile, SharedStore,
        assert_name_unique, collection_dir_path, find_unique_name, folder_dir_name,
        persist_shared_tree, request_file_name, request_file_path, root_collection_id_of,
        scope_key, shared_store_from_collection_manifest_path,
    };
    use std::collections::HashMap;
    use tempfile::tempdir;

    use crate::models::{
        AuthConfig, BodyConfig, HttpMethod, RequestDefinition, RequestFile, RequestMeta,
        ScriptConfig,
    };
    use crate::paths::BeamPaths;
    use chrono::Utc;

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
                        description: None,
                        created_at: None,
                        updated_at: None,
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
                        description: None,
                        created_at: None,
                        updated_at: None,
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
                        description: None,
                        created_at: None,
                        updated_at: None,
                        parent_id: Some(folder_id),
                        children: Vec::new(),
                    },
                ),
            ]),
            requests: HashMap::new(),
            root_ids: vec![collection_id],
            name_index: HashMap::new(),
            environments: HashMap::new(),
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
                        description: None,
                        created_at: None,
                        updated_at: None,
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
                        description: None,
                        created_at: None,
                        updated_at: None,
                        parent_id: Some(parent_id),
                        children: Vec::new(),
                    },
                ),
            ]),
            requests: HashMap::new(),
            root_ids: Vec::new(),
            name_index: HashMap::new(),
            environments: HashMap::new(),
        };

        let warnings = store.rebuild_name_index();
        assert_eq!(warnings.len(), 1);
        assert_eq!(store.name_index.len(), 1);
    }

    #[test]
    fn collection_manifest_from_store_preserves_metadata_and_child_order() {
        let collection_id = ulid::Ulid::new();
        let folder_id = ulid::Ulid::new();
        let request_id = ulid::Ulid::new();
        let direct_request_id = ulid::Ulid::new();
        let created_at = Utc::now();
        let updated_at = Utc::now();
        let store = SharedStore {
            nodes: HashMap::from([
                (
                    collection_id,
                    Node {
                        id: collection_id,
                        name: "Sample API".to_string(),
                        kind: NodeKind::Collection,
                        description: Some("Collection".to_string()),
                        created_at: Some(created_at),
                        updated_at: Some(updated_at),
                        parent_id: None,
                        children: vec![folder_id, direct_request_id],
                    },
                ),
                (
                    folder_id,
                    Node {
                        id: folder_id,
                        name: "Users".to_string(),
                        kind: NodeKind::Folder,
                        description: Some("Folder".to_string()),
                        created_at: Some(created_at),
                        updated_at: Some(updated_at),
                        parent_id: Some(collection_id),
                        children: vec![request_id],
                    },
                ),
                (
                    request_id,
                    Node {
                        id: request_id,
                        name: "List Users".to_string(),
                        kind: NodeKind::Request,
                        description: Some("Nested request".to_string()),
                        created_at: Some(created_at),
                        updated_at: Some(updated_at),
                        parent_id: Some(folder_id),
                        children: Vec::new(),
                    },
                ),
                (
                    direct_request_id,
                    Node {
                        id: direct_request_id,
                        name: "Health".to_string(),
                        kind: NodeKind::Request,
                        description: None,
                        created_at: Some(created_at),
                        updated_at: Some(updated_at),
                        parent_id: Some(collection_id),
                        children: Vec::new(),
                    },
                ),
            ]),
            requests: HashMap::new(),
            root_ids: vec![collection_id],
            name_index: HashMap::new(),
            environments: HashMap::new(),
        };

        let manifest =
            super::collection_manifest_from_store(&store, collection_id).expect("build manifest");

        assert_eq!(manifest.description.as_deref(), Some("Collection"));
        assert_eq!(manifest.children.len(), 2);
        assert_eq!(manifest.children[0].id, folder_id);
        assert_eq!(manifest.children[0].description.as_deref(), Some("Folder"));
        assert_eq!(manifest.children[0].children[0].id, request_id);
        assert_eq!(manifest.children[1].id, direct_request_id);
    }

    #[test]
    fn shared_store_from_manifest_flattens_nodes_and_preserves_order() {
        let dir = tempdir().expect("tempdir");
        let manifest_path = dir.path().join(COLLECTION_MANIFEST_FILE_NAME);
        let collection_id = ulid::Ulid::new();
        let folder_id = ulid::Ulid::new();
        let request_id = ulid::Ulid::new();
        let direct_request_id = ulid::Ulid::new();
        let manifest = CollectionManifestFile {
            schema_version: crate::schema::SCHEMA_VERSION_V1,
            id: collection_id,
            name: "Sample".to_string(),
            kind: NodeKind::Collection,
            description: Some("Collection".to_string()),
            created_at: None,
            updated_at: None,
            children: vec![
                ManifestNode {
                    id: folder_id,
                    name: "Nested".to_string(),
                    kind: NodeKind::Folder,
                    description: None,
                    created_at: None,
                    updated_at: None,
                    children: vec![ManifestNode {
                        id: request_id,
                        name: "Get Data".to_string(),
                        kind: NodeKind::Request,
                        description: None,
                        created_at: None,
                        updated_at: None,
                        children: Vec::new(),
                    }],
                },
                ManifestNode {
                    id: direct_request_id,
                    name: "Health".to_string(),
                    kind: NodeKind::Request,
                    description: None,
                    created_at: None,
                    updated_at: None,
                    children: Vec::new(),
                },
            ],
        };
        std::fs::write(
            &manifest_path,
            toml::to_string_pretty(&manifest).expect("encode manifest"),
        )
        .expect("write manifest");

        let store =
            shared_store_from_collection_manifest_path(&manifest_path).expect("load shared store");
        assert_eq!(store.root_ids, vec![collection_id]);
        assert_eq!(store.nodes.len(), 4);
        assert_eq!(
            store
                .nodes
                .get(&collection_id)
                .expect("collection")
                .children,
            vec![folder_id, direct_request_id]
        );
        assert_eq!(
            store.nodes.get(&folder_id).expect("folder").children,
            vec![request_id]
        );
        assert_eq!(
            store.nodes.get(&request_id).expect("request").parent_id,
            Some(folder_id)
        );
        assert_eq!(
            store
                .name_index
                .get(&scope_key(Some(folder_id), "Get Data")),
            Some(&request_id)
        );
    }

    #[test]
    fn path_resolution_uses_node_ancestry() {
        let dir = tempdir().expect("tempdir");
        let paths = BeamPaths::from_root(dir.path().join("beam"));
        let collection_id = ulid::Ulid::new();
        let folder_id = ulid::Ulid::new();
        let request_id = ulid::Ulid::new();
        let store = SharedStore {
            nodes: HashMap::from([
                (
                    collection_id,
                    Node {
                        id: collection_id,
                        name: "Sample API".to_string(),
                        kind: NodeKind::Collection,
                        description: None,
                        created_at: None,
                        updated_at: None,
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
                        description: None,
                        created_at: None,
                        updated_at: None,
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
                        description: None,
                        created_at: None,
                        updated_at: None,
                        parent_id: Some(folder_id),
                        children: Vec::new(),
                    },
                ),
            ]),
            requests: HashMap::new(),
            root_ids: vec![collection_id],
            name_index: HashMap::new(),
            environments: HashMap::new(),
        };

        assert_eq!(
            collection_dir_path(&paths, &store, collection_id).expect("collection path"),
            paths.collections_dir.join("sample-api")
        );
        assert_eq!(folder_dir_name("Users"), "users");
        assert_eq!(
            request_file_path(&paths, &store, request_id).expect("request path"),
            paths
                .collections_dir
                .join("sample-api")
                .join("users")
                .join(request_file_name("Get User"))
        );
    }

    #[test]
    fn persist_shared_tree_writes_root_order_manifest_and_requests() {
        let dir = tempdir().expect("tempdir");
        let paths = BeamPaths::from_root(dir.path().join("beam"));
        let first_collection_id = ulid::Ulid::new();
        let second_collection_id = ulid::Ulid::new();
        let folder_id = ulid::Ulid::new();
        let request_id = ulid::Ulid::new();
        let now = Utc::now();

        let request_file = RequestFile {
            meta: RequestMeta {
                request_id,
                name: "Outdated Name".to_string(),
                description: Some("Request description".to_string()),
                created_at: now,
                updated_at: now,
            },
            request: RequestDefinition {
                method: HttpMethod::Get,
                url: "https://example.com/users".to_string(),
                headers: Vec::new(),
                query_params: Vec::new(),
            },
            auth: AuthConfig::None,
            body: BodyConfig::None,
            scripts: ScriptConfig::default(),
            file_path: None,
        };
        let store = SharedStore {
            nodes: HashMap::from([
                (
                    first_collection_id,
                    Node {
                        id: first_collection_id,
                        name: "First".to_string(),
                        kind: NodeKind::Collection,
                        description: Some("First collection".to_string()),
                        created_at: Some(now),
                        updated_at: Some(now),
                        parent_id: None,
                        children: Vec::new(),
                    },
                ),
                (
                    second_collection_id,
                    Node {
                        id: second_collection_id,
                        name: "Second".to_string(),
                        kind: NodeKind::Collection,
                        description: Some("Second collection".to_string()),
                        created_at: Some(now),
                        updated_at: Some(now),
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
                        description: Some("Folder".to_string()),
                        created_at: Some(now),
                        updated_at: Some(now),
                        parent_id: Some(second_collection_id),
                        children: vec![request_id],
                    },
                ),
                (
                    request_id,
                    Node {
                        id: request_id,
                        name: "Get Users".to_string(),
                        kind: NodeKind::Request,
                        description: Some("Request node".to_string()),
                        created_at: Some(now),
                        updated_at: Some(now),
                        parent_id: Some(folder_id),
                        children: Vec::new(),
                    },
                ),
            ]),
            requests: HashMap::from([(request_id, request_file)]),
            root_ids: vec![second_collection_id, first_collection_id],
            name_index: HashMap::new(),
            environments: HashMap::new(),
        };

        persist_shared_tree(&paths, &store).expect("persist shared tree");

        let manifest_path = paths
            .collections_dir
            .join("second")
            .join(COLLECTION_MANIFEST_FILE_NAME);
        let encoded_manifest = std::fs::read_to_string(&manifest_path).expect("read manifest");
        let manifest: CollectionManifestFile =
            toml::from_str(&encoded_manifest).expect("decode manifest");
        assert_eq!(manifest.id, second_collection_id);
        assert_eq!(manifest.children[0].id, folder_id);
        assert_eq!(manifest.children[0].children[0].id, request_id);

        let request_path = paths
            .collections_dir
            .join("second")
            .join("users")
            .join(request_file_name("Get Users"));
        let encoded_request = std::fs::read_to_string(&request_path).expect("read request");
        let persisted_request: RequestFile =
            toml::from_str(&encoded_request).expect("decode request");
        assert_eq!(persisted_request.meta.name, "Get Users");

        let encoded_root_order =
            std::fs::read_to_string(&paths.collections_root_order_file).expect("read root order");
        let root_order: RootOrderFile =
            toml::from_str(&encoded_root_order).expect("decode root order");
        assert_eq!(
            root_order.root_ids,
            vec![second_collection_id, first_collection_id]
        );
    }
}
