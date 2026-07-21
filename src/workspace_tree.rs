use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use ulid::Ulid;

use crate::error::{BeamError, Result};
use crate::models::{EnvironmentFile, RequestFile};
use crate::paths::BeamPaths;

pub type NodeId = Ulid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeKind {
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

pub fn ensure_parent_kind(store: &SharedStore, parent_id: NodeId) -> Result<NodeKind> {
    let parent = store
        .nodes
        .get(&parent_id)
        .ok_or_else(|| BeamError::NotFound {
            entity: "parent_node",
            id: parent_id.to_string(),
        })?;
    match parent.kind {
        NodeKind::Folder => Ok(parent.kind),
        NodeKind::Request => Err(BeamError::Validation {
            message: format!("request node {parent_id} cannot accept child nodes"),
        }),
    }
}

pub fn apply_child_move(
    store: &mut SharedStore,
    child_id: NodeId,
    source_parent_id: NodeId,
    destination_parent_id: NodeId,
    insertion_index: usize,
) -> Result<()> {
    let removed_index = {
        let source_parent =
            store
                .nodes
                .get_mut(&source_parent_id)
                .ok_or_else(|| BeamError::NotFound {
                    entity: "source_parent",
                    id: source_parent_id.to_string(),
                })?;
        let index = source_parent
            .children
            .iter()
            .position(|id| *id == child_id)
            .ok_or_else(|| BeamError::NotFound {
                entity: "child_in_source_parent",
                id: child_id.to_string(),
            })?;
        source_parent.children.remove(index);
        index
    };

    let adjusted_index =
        if source_parent_id == destination_parent_id && removed_index < insertion_index {
            insertion_index.saturating_sub(1)
        } else {
            insertion_index
        };

    let destination_parent =
        store
            .nodes
            .get_mut(&destination_parent_id)
            .ok_or_else(|| BeamError::NotFound {
                entity: "destination_parent",
                id: destination_parent_id.to_string(),
            })?;
    let index = adjusted_index.min(destination_parent.children.len());
    destination_parent.children.insert(index, child_id);
    Ok(())
}

fn normalize_name(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        "Untitled".to_string()
    } else {
        trimmed.to_string()
    }
}

const MAX_SLUG_LEN: usize = 80;
const SLUG_HASH_SUFFIX_LEN: usize = 8;

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
    let slug = out.trim_matches('-').to_string();
    let slug = if slug.is_empty() {
        "untitled".to_string()
    } else {
        slug
    };

    if slug.len() <= MAX_SLUG_LEN {
        slug
    } else {
        // File systems cap file-name length (NAME_MAX is 255 bytes on most
        // platforms, and we still need room for extensions like `.request.toml.tmp`).
        // Shorten the slug and append a stable short hash of the full slug so
        // distinct long names keep producing distinct on-disk names.
        let hash = fnv1a_hex(&slug, SLUG_HASH_SUFFIX_LEN);
        let stem_budget = MAX_SLUG_LEN - SLUG_HASH_SUFFIX_LEN - 1; // 1 for '-' separator
        let mut stem: String = slug.chars().take(stem_budget).collect();
        // Drop any trailing '-' so we don't end up with `...--<hash>`.
        while stem.ends_with('-') {
            stem.pop();
        }
        if stem.is_empty() {
            format!("u-{}", hash)
        } else {
            format!("{}-{}", stem, hash)
        }
    }
}

fn fnv1a_hex(input: &str, hex_len: usize) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in input.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{:016x}", h).chars().take(hex_len).collect()
}

pub fn folder_dir_name(name: &str) -> String {
    slugify_name(name)
}

pub fn request_file_name(name: &str) -> String {
    format!("{}.request.toml", slugify_name(name))
}

/// Returns the directory path for a folder node.
/// For top-level folders (parent_id: None), the parent is the workspace root.
pub fn folder_dir_path(
    paths: &BeamPaths,
    store: &SharedStore,
    folder_id: NodeId,
) -> Result<PathBuf> {
    let folder = node_by_kind(store, folder_id, NodeKind::Folder)?;
    match folder.parent_id {
        None => Ok(paths.root.join(folder_dir_name(&folder.name))),
        Some(parent_id) => {
            Ok(node_dir_path(paths, store, parent_id)?.join(folder_dir_name(&folder.name)))
        }
    }
}

pub fn node_dir_path(paths: &BeamPaths, store: &SharedStore, node_id: NodeId) -> Result<PathBuf> {
    match node_by_id(store, node_id)?.kind {
        NodeKind::Folder => folder_dir_path(paths, store, node_id),
        NodeKind::Request => Err(BeamError::Validation {
            message: format!("request node {} does not have a directory path", node_id),
        }),
    }
}

/// Returns the file path for a request node.
/// For root-level requests (parent_id: None), the file lives directly in the workspace root.
pub fn request_file_path(
    paths: &BeamPaths,
    store: &SharedStore,
    request_id: NodeId,
) -> Result<PathBuf> {
    let request = node_by_kind(store, request_id, NodeKind::Request)?;
    let request_dir = match request.parent_id {
        None => paths.root.clone(),
        Some(parent_id) => {
            let parent = node_by_id(store, parent_id)?;
            match parent.kind {
                NodeKind::Folder => node_dir_path(paths, store, parent_id)?,
                NodeKind::Request => {
                    return Err(BeamError::Validation {
                        message: format!(
                            "request node {} cannot be parent of request node {}",
                            parent_id, request_id
                        ),
                    });
                }
            }
        }
    };
    Ok(request_dir.join(request_file_name(&request.name)))
}

pub fn node_by_id(store: &SharedStore, node_id: NodeId) -> Result<&Node> {
    store
        .nodes
        .get(&node_id)
        .ok_or_else(|| BeamError::NotFound {
            entity: "node",
            id: node_id.to_string(),
        })
}

pub fn node_by_kind(
    store: &SharedStore,
    node_id: NodeId,
    expected_kind: NodeKind,
) -> Result<&Node> {
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

#[cfg(test)]
mod tests {
    use super::{
        MAX_SLUG_LEN, NameValidationError, Node, NodeKind, SLUG_HASH_SUFFIX_LEN, SharedStore,
        assert_name_unique, find_unique_name, folder_dir_name, request_file_name,
        request_file_path, scope_key,
    };
    use std::collections::HashMap;
    use tempfile::tempdir;

    use crate::paths::BeamPaths;

    #[test]
    fn scope_key_uses_root_or_parent_scope_with_slugified_name() {
        let parent_id = ulid::Ulid::new();
        assert_eq!(scope_key(None, "My Folder"), "root/my-folder");
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
    fn path_resolution_uses_node_ancestry() {
        let dir = tempdir().expect("tempdir");
        let paths = BeamPaths::from_root(dir.path().join("beam"));
        let folder_id = ulid::Ulid::new();
        let request_id = ulid::Ulid::new();
        let store = SharedStore {
            nodes: HashMap::from([
                (
                    folder_id,
                    Node {
                        id: folder_id,
                        name: "Users".to_string(),
                        kind: NodeKind::Folder,
                        description: None,
                        created_at: None,
                        updated_at: None,
                        parent_id: None,
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
            root_ids: vec![folder_id],
            name_index: HashMap::new(),
            environments: HashMap::new(),
        };

        assert_eq!(folder_dir_name("Users"), "users");
        assert_eq!(
            request_file_path(&paths, &store, request_id).expect("request path"),
            paths.root.join("users").join(request_file_name("Get User"))
        );
    }

    #[test]
    fn slugify_name_truncates_long_names_with_stable_hash_suffix() {
        let long_input = "a".repeat(500);
        let slug = folder_dir_name(&long_input);
        assert!(slug.len() <= MAX_SLUG_LEN);
        assert!(slug.len() > SLUG_HASH_SUFFIX_LEN);
        let parts: Vec<&str> = slug.rsplitn(2, '-').collect();
        assert_eq!(parts[0].len(), SLUG_HASH_SUFFIX_LEN);
        // Same input produces the same slug (stable across reloads).
        assert_eq!(slug, folder_dir_name(&long_input));
        // Distinct long inputs produce distinct shortened slugs.
        let other = "b".repeat(500);
        assert_ne!(slug, folder_dir_name(&other));
    }

    #[test]
    fn request_file_name_stays_under_filesystem_limit_for_long_names() {
        let long_input = "x".repeat(1000);
        let file_name = request_file_name(&long_input);
        assert!(file_name.len() <= MAX_SLUG_LEN + ".request.toml".len());
    }

    #[test]
    fn root_level_request_path_is_directly_under_workspace_root() {
        let dir = tempdir().expect("tempdir");
        let paths = BeamPaths::from_root(dir.path().join("beam"));
        let request_id = ulid::Ulid::new();
        let store = SharedStore {
            nodes: HashMap::from([(
                request_id,
                Node {
                    id: request_id,
                    name: "Health Check".to_string(),
                    kind: NodeKind::Request,
                    description: None,
                    created_at: None,
                    updated_at: None,
                    parent_id: None,
                    children: Vec::new(),
                },
            )]),
            requests: HashMap::new(),
            root_ids: vec![request_id],
            name_index: HashMap::new(),
            environments: HashMap::new(),
        };

        assert_eq!(
            request_file_path(&paths, &store, request_id).expect("request path"),
            paths.root.join(request_file_name("Health Check"))
        );
    }
}
