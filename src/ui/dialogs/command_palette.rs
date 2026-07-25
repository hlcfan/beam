use std::collections::HashMap;

use ulid::Ulid;

use crate::app_shell::{TreeNodeKind, WorkspaceTreeDescriptor, WorkspaceTreeState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
pub(super) enum CommandPaletteCommand {
    OpenSettings,
    OpenKeyBindings,
    OpenEnvironmentManager,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CommandPaletteEntryKind {
    Request { request_id: Ulid },
    Folder { folder_id: Ulid },
    Command(CommandPaletteCommand),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CommandPaletteEntry {
    pub(super) kind: CommandPaletteEntryKind,
    pub(super) title: String,
    pub(super) subtitle: Option<String>,
    pub(super) search_name: String,
    pub(super) request_recency_rank: Option<usize>,
}

const COMMANDS: [(CommandPaletteCommand, &str); 3] = [
    (CommandPaletteCommand::OpenSettings, "Open Settings"),
    (CommandPaletteCommand::OpenKeyBindings, "Open Key Bindings"),
    (
        CommandPaletteCommand::OpenEnvironmentManager,
        "Open Environment Manager",
    ),
];

pub(super) fn build_command_palette_entries(
    tree: &WorkspaceTreeState,
    recent_request_ids: &[Ulid],
) -> Vec<CommandPaletteEntry> {
    build_entries_from_descriptors(tree.preorder_descriptors(), recent_request_ids)
}

fn build_entries_from_descriptors(
    descriptors: Vec<WorkspaceTreeDescriptor>,
    recent_request_ids: &[Ulid],
) -> Vec<CommandPaletteEntry> {
    let mut recency_by_id = HashMap::new();
    for (rank, request_id) in recent_request_ids.iter().copied().enumerate() {
        recency_by_id.entry(request_id).or_insert(rank);
    }
    let mut entries = descriptors
        .into_iter()
        .map(|descriptor| {
            let subtitle = (!descriptor.ancestor_path.is_empty())
                .then(|| descriptor.ancestor_path.join(" / "));
            let (kind, request_recency_rank) = match descriptor.kind {
                TreeNodeKind::Request => (
                    CommandPaletteEntryKind::Request {
                        request_id: descriptor.id,
                    },
                    recency_by_id.get(&descriptor.id).copied(),
                ),
                TreeNodeKind::Folder => (
                    CommandPaletteEntryKind::Folder {
                        folder_id: descriptor.id,
                    },
                    None,
                ),
            };
            CommandPaletteEntry {
                kind,
                search_name: descriptor.name.to_lowercase(),
                title: descriptor.name,
                subtitle,
                request_recency_rank,
            }
        })
        .collect::<Vec<_>>();

    entries.extend(COMMANDS.map(|(command, title)| CommandPaletteEntry {
        kind: CommandPaletteEntryKind::Command(command),
        title: title.to_string(),
        subtitle: None,
        search_name: title.to_lowercase(),
        request_recency_rank: None,
    }));
    entries
}

pub(super) fn filter_command_palette_entries(
    entries: &[CommandPaletteEntry],
    query: &str,
) -> Vec<CommandPaletteEntry> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        let mut recent = entries
            .iter()
            .filter(|entry| entry.request_recency_rank.is_some())
            .cloned()
            .collect::<Vec<_>>();
        recent.sort_by_key(|entry| entry.request_recency_rank);
        recent.extend(
            entries
                .iter()
                .filter(|entry| matches!(entry.kind, CommandPaletteEntryKind::Command(_)))
                .cloned(),
        );
        return recent;
    }

    let mut matches = entries
        .iter()
        .filter(|entry| entry.search_name.contains(&query))
        .cloned()
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| {
        match_tier(left, &query)
            .cmp(&match_tier(right, &query))
            .then_with(
                || match (left.request_recency_rank, right.request_recency_rank) {
                    (Some(left_rank), Some(right_rank)) => left_rank.cmp(&right_rank),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                },
            )
    });
    matches
}

fn match_tier(entry: &CommandPaletteEntry, query: &str) -> u8 {
    if entry.search_name == query {
        0
    } else if entry.search_name.starts_with(query) {
        1
    } else {
        2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(
        id: Ulid,
        kind: TreeNodeKind,
        name: &str,
        ancestor_path: &[&str],
    ) -> WorkspaceTreeDescriptor {
        WorkspaceTreeDescriptor {
            id,
            kind,
            name: name.to_string(),
            ancestor_path: ancestor_path
                .iter()
                .map(|segment| (*segment).to_string())
                .collect(),
        }
    }

    fn entry(
        title: &str,
        kind: CommandPaletteEntryKind,
        recency: Option<usize>,
    ) -> CommandPaletteEntry {
        CommandPaletteEntry {
            kind,
            title: title.to_string(),
            subtitle: None,
            search_name: title.to_lowercase(),
            request_recency_rank: recency,
        }
    }

    fn titles(entries: &[CommandPaletteEntry]) -> Vec<&str> {
        entries.iter().map(|entry| entry.title.as_str()).collect()
    }

    #[test]
    fn builds_tree_entries_in_order_with_paths_and_normalized_names() {
        let folder = Ulid::new();
        let nested_folder = Ulid::new();
        let request = Ulid::new();
        let entries = build_entries_from_descriptors(
            vec![
                descriptor(folder, TreeNodeKind::Folder, "Users", &[]),
                descriptor(
                    nested_folder,
                    TreeNodeKind::Folder,
                    "Authentication",
                    &["Users"],
                ),
                descriptor(
                    request,
                    TreeNodeKind::Request,
                    "Get PROFILE",
                    &["Users", "Authentication"],
                ),
            ],
            &[request],
        );

        assert_eq!(
            titles(&entries),
            vec![
                "Users",
                "Authentication",
                "Get PROFILE",
                "Open Settings",
                "Open Key Bindings",
                "Open Environment Manager"
            ]
        );
        assert_eq!(entries[0].subtitle, None);
        assert_eq!(entries[1].subtitle.as_deref(), Some("Users"));
        assert_eq!(
            entries[2].subtitle.as_deref(),
            Some("Users / Authentication")
        );
        assert_eq!(entries[2].search_name, "get profile");
        assert_eq!(entries[2].request_recency_rank, Some(0));
    }

    #[test]
    fn stale_history_ids_are_ignored() {
        let stale = Ulid::new();
        let request = Ulid::new();
        let entries = build_entries_from_descriptors(
            vec![descriptor(request, TreeNodeKind::Request, "Current", &[])],
            &[stale],
        );

        assert_eq!(entries[0].request_recency_rank, None);
        assert_eq!(
            titles(&filter_command_palette_entries(&entries, "")),
            vec![
                "Open Settings",
                "Open Key Bindings",
                "Open Environment Manager"
            ]
        );
    }

    #[test]
    fn empty_query_shows_recent_requests_then_commands_only() {
        let older = Ulid::new();
        let newer = Ulid::new();
        let folder = Ulid::new();
        let unvisited = Ulid::new();
        let entries = vec![
            entry(
                "Older",
                CommandPaletteEntryKind::Request { request_id: older },
                Some(1),
            ),
            entry(
                "Folder",
                CommandPaletteEntryKind::Folder { folder_id: folder },
                None,
            ),
            entry(
                "Unvisited",
                CommandPaletteEntryKind::Request {
                    request_id: unvisited,
                },
                None,
            ),
            entry(
                "Newer",
                CommandPaletteEntryKind::Request { request_id: newer },
                Some(0),
            ),
            entry(
                "Open Settings",
                CommandPaletteEntryKind::Command(CommandPaletteCommand::OpenSettings),
                None,
            ),
        ];

        assert_eq!(
            titles(&filter_command_palette_entries(&entries, "  ")),
            vec!["Newer", "Older", "Open Settings"]
        );
    }

    #[test]
    fn ranks_exact_then_prefix_then_substring_matches_case_insensitively() {
        let entries = vec![
            entry(
                "My Get User Request",
                CommandPaletteEntryKind::Request {
                    request_id: Ulid::new(),
                },
                None,
            ),
            entry(
                "Get User Details",
                CommandPaletteEntryKind::Request {
                    request_id: Ulid::new(),
                },
                None,
            ),
            entry(
                "GET USER",
                CommandPaletteEntryKind::Request {
                    request_id: Ulid::new(),
                },
                None,
            ),
        ];

        assert_eq!(
            titles(&filter_command_palette_entries(&entries, "  gEt UsEr ")),
            vec!["GET USER", "Get User Details", "My Get User Request"]
        );
    }

    #[test]
    fn recency_breaks_ties_between_matching_requests() {
        let entries = vec![
            entry(
                "Get Older",
                CommandPaletteEntryKind::Request {
                    request_id: Ulid::new(),
                },
                Some(4),
            ),
            entry(
                "Get Unvisited",
                CommandPaletteEntryKind::Request {
                    request_id: Ulid::new(),
                },
                None,
            ),
            entry(
                "Get Newer",
                CommandPaletteEntryKind::Request {
                    request_id: Ulid::new(),
                },
                Some(0),
            ),
        ];

        assert_eq!(
            titles(&filter_command_palette_entries(&entries, "get")),
            vec!["Get Newer", "Get Older", "Get Unvisited"]
        );
    }

    #[test]
    fn duplicate_names_preserve_tree_order_when_recency_is_equal() {
        let entries = vec![
            entry(
                "Health",
                CommandPaletteEntryKind::Request {
                    request_id: Ulid::new(),
                },
                None,
            ),
            entry(
                "Health",
                CommandPaletteEntryKind::Request {
                    request_id: Ulid::new(),
                },
                None,
            ),
        ];
        let filtered = filter_command_palette_entries(&entries, "health");

        assert_eq!(filtered, entries);
    }

    #[test]
    fn folder_name_matches_but_ancestor_subtitle_does_not() {
        let folder_id = Ulid::new();
        let request_id = Ulid::new();
        let mut folder = entry(
            "Authentication",
            CommandPaletteEntryKind::Folder { folder_id },
            None,
        );
        folder.subtitle = Some("Users".to_string());
        let mut request = entry(
            "Profile",
            CommandPaletteEntryKind::Request { request_id },
            None,
        );
        request.subtitle = Some("Users / Authentication".to_string());
        let entries = vec![folder, request];

        assert_eq!(
            titles(&filter_command_palette_entries(&entries, "authentication")),
            vec!["Authentication"]
        );
        assert!(filter_command_palette_entries(&entries, "users").is_empty());
    }

    #[test]
    fn returns_no_results_when_nothing_matches() {
        let entries = vec![entry(
            "Health",
            CommandPaletteEntryKind::Request {
                request_id: Ulid::new(),
            },
            None,
        )];

        assert!(filter_command_palette_entries(&entries, "missing").is_empty());
    }
}
