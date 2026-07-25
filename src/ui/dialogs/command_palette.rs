use std::collections::HashMap;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, ScrollHandle, StatefulInteractiveElement as _, Styled as _, Subscription, Window,
    actions, div, px,
};
use gpui_component::{
    ActiveTheme as _, Icon, Sizable as _, WindowExt as _, h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use ulid::Ulid;

use super::super::{BeamView, OpenCommandPalette};
use crate::app_shell::{TreeNodeKind, WorkspaceTreeDescriptor, WorkspaceTreeState};

const COMMAND_PALETTE_CONTEXT: &str = "CommandPalette";
const RESULT_ROW_HEIGHT: f32 = 48.0;
const RESULT_SECTION_HEIGHT: f32 = 28.0;
const MAX_RESULT_LIST_HEIGHT: f32 = 380.0;
const MAX_RECENT_REQUESTS: usize = 10;

actions!(
    command_palette,
    [
        SelectNextPaletteItem,
        SelectPreviousPaletteItem,
        ConfirmPaletteItem,
        DismissCommandPalette
    ]
);

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
        recent.truncate(MAX_RECENT_REQUESTS);
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

fn match_tier(entry: &CommandPaletteEntry, query: &str) -> u8 {
    if entry.search_name == query {
        0
    } else if entry.search_name.starts_with(query) {
        1
    } else {
        2
    }
}

fn next_selection_index(selected_index: usize, entry_count: usize) -> usize {
    if entry_count == 0 {
        return reset_selection_index();
    }
    selected_index.saturating_add(1) % entry_count
}

fn previous_selection_index(selected_index: usize, entry_count: usize) -> usize {
    if entry_count == 0 {
        return reset_selection_index();
    }
    if selected_index == 0 || selected_index >= entry_count {
        entry_count - 1
    } else {
        selected_index - 1
    }
}

fn reset_selection_index() -> usize {
    0
}

pub(in crate::ui) struct CommandPaletteDialogView {
    beam_view: Entity<BeamView>,
    search_input: Entity<InputState>,
    all_entries: Vec<CommandPaletteEntry>,
    filtered_entries: Vec<CommandPaletteEntry>,
    selected_index: usize,
    result_scroll_handle: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl CommandPaletteDialogView {
    pub(super) fn new(
        beam_view: Entity<BeamView>,
        all_entries: Vec<CommandPaletteEntry>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Search requests, folders, and commands…")
        });
        let filtered_entries = filter_command_palette_entries(&all_entries, "");
        let input_for_subscription = search_input.clone();
        let subscription = cx.subscribe_in(
            &search_input,
            window,
            move |this, _, event: &InputEvent, _, cx| {
                if !matches!(event, InputEvent::Change) {
                    return;
                }
                let query = input_for_subscription.read(cx).value().to_string();
                this.filtered_entries = filter_command_palette_entries(&this.all_entries, &query);
                this.selected_index = reset_selection_index();
                if !this.filtered_entries.is_empty() {
                    this.result_scroll_handle
                        .scroll_to_item(this.selected_index);
                }
                cx.notify();
            },
        );

        Self {
            beam_view,
            search_input,
            all_entries,
            filtered_entries,
            selected_index: reset_selection_index(),
            result_scroll_handle: ScrollHandle::new(),
            _subscriptions: vec![subscription],
        }
    }

    pub(super) fn focus_search_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_input
            .update(cx, |input, cx| input.focus(window, cx));
    }

    fn select_next(&mut self, cx: &mut Context<Self>) {
        if self.filtered_entries.is_empty() {
            return;
        }
        self.selected_index =
            next_selection_index(self.selected_index, self.filtered_entries.len());
        self.result_scroll_handle
            .scroll_to_item(self.selected_index);
        cx.notify();
    }

    fn select_previous(&mut self, cx: &mut Context<Self>) {
        if self.filtered_entries.is_empty() {
            return;
        }
        self.selected_index =
            previous_selection_index(self.selected_index, self.filtered_entries.len());
        self.result_scroll_handle
            .scroll_to_item(self.selected_index);
        cx.notify();
    }

    fn activate_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.activate_index(self.selected_index, window, cx);
    }

    fn activate_index(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.filtered_entries.get(index).cloned() else {
            return;
        };
        self.selected_index = index;
        window.close_dialog(cx);

        self.beam_view.update(cx, |beam_view, cx| {
            beam_view.command_palette_dialog_view = None;
            match entry.kind {
                CommandPaletteEntryKind::Request { request_id } => {
                    if !beam_view
                        .shell
                        .workspace_tree
                        .node(request_id)
                        .is_some_and(|node| node.kind == TreeNodeKind::Request)
                    {
                        return;
                    }
                    beam_view.select_request(request_id, window, cx);
                    beam_view.commit_request_selection(window, cx);
                }
                CommandPaletteEntryKind::Folder { folder_id } => {
                    beam_view.reveal_tree_folder(folder_id, window, cx);
                }
                CommandPaletteEntryKind::Command(CommandPaletteCommand::OpenSettings) => {
                    beam_view.open_settings_dialog(window, cx);
                }
                CommandPaletteEntryKind::Command(CommandPaletteCommand::OpenKeyBindings) => {
                    beam_view.open_key_bindings_dialog(window, cx);
                }
                CommandPaletteEntryKind::Command(CommandPaletteCommand::OpenEnvironmentManager) => {
                    beam_view.open_environment_manager(window, cx);
                }
            }
        });
    }

    fn on_select_next(
        &mut self,
        _: &SelectNextPaletteItem,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        self.select_next(cx);
    }

    fn on_select_previous(
        &mut self,
        _: &SelectPreviousPaletteItem,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        self.select_previous(cx);
    }

    fn on_confirm(&mut self, _: &ConfirmPaletteItem, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.activate_selected(window, cx);
    }

    fn on_dismiss(
        &mut self,
        _: &DismissCommandPalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        window.close_dialog(cx);
        self.beam_view.update(cx, |beam_view, cx| {
            beam_view.command_palette_dialog_view = None;
            cx.notify();
        });
    }

    fn on_open_palette(&mut self, _: &OpenCommandPalette, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
    }

    fn icon_path(entry: &CommandPaletteEntry) -> &'static str {
        match entry.kind {
            CommandPaletteEntryKind::Request { .. } => "icons/file.svg",
            CommandPaletteEntryKind::Folder { .. } => "icons/folder.svg",
            CommandPaletteEntryKind::Command(CommandPaletteCommand::OpenSettings) => {
                "icons/settings.svg"
            }
            CommandPaletteEntryKind::Command(CommandPaletteCommand::OpenKeyBindings) => {
                "icons/command.svg"
            }
            CommandPaletteEntryKind::Command(CommandPaletteCommand::OpenEnvironmentManager) => {
                "icons/variable.svg"
            }
        }
    }

    fn section_label(
        entry: &CommandPaletteEntry,
        previous: Option<&CommandPaletteEntry>,
        has_query: bool,
    ) -> Option<&'static str> {
        if has_query {
            return previous.is_none().then_some("RESULTS");
        }

        match (&entry.kind, previous.map(|entry| &entry.kind)) {
            (CommandPaletteEntryKind::Request { .. }, None) => Some("RECENT"),
            (CommandPaletteEntryKind::Command(_), None)
            | (
                CommandPaletteEntryKind::Command(_),
                Some(CommandPaletteEntryKind::Request { .. }),
            ) => Some("COMMANDS"),
            _ => None,
        }
    }

    fn keycap(label: &'static str, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_1p5()
            .py_0p5()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().muted)
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(label)
    }
}

impl Render for CommandPaletteDialogView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _ = &self.beam_view;
        let query = self.search_input.read(cx).value().trim().to_string();
        let has_query = !query.is_empty();
        let section_count = if self.filtered_entries.is_empty() {
            0
        } else if has_query {
            1
        } else {
            1 + usize::from(
                self.filtered_entries
                    .iter()
                    .any(|entry| matches!(entry.kind, CommandPaletteEntryKind::Request { .. })),
            )
        };
        let list_height = px(((self.filtered_entries.len() as f32 * RESULT_ROW_HEIGHT)
            + section_count as f32 * RESULT_SECTION_HEIGHT)
            .min(MAX_RESULT_LIST_HEIGHT));
        let mut results = v_flex()
            .id("command-palette-results")
            .w_full()
            .h(list_height)
            .overflow_y_scroll()
            .track_scroll(&self.result_scroll_handle);

        for (index, entry) in self.filtered_entries.iter().cloned().enumerate() {
            let is_selected = index == self.selected_index;
            let section_label = Self::section_label(
                &entry,
                index
                    .checked_sub(1)
                    .and_then(|index| self.filtered_entries.get(index)),
                has_query,
            );
            let icon_path = Self::icon_path(&entry);
            let title = entry.title;
            let subtitle = entry.subtitle;
            let row = h_flex()
                .id(("command-palette-row", index))
                .w_full()
                .h(px(RESULT_ROW_HEIGHT))
                .flex_none()
                .items_center()
                .gap_3()
                .px_3()
                .rounded(cx.theme().radius)
                .cursor_pointer()
                .when(is_selected, |this| this.bg(cx.theme().list_active))
                .when(!is_selected, |this| {
                    this.hover(|this| this.bg(cx.theme().list_hover))
                })
                .child(
                    div()
                        .size_7()
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .rounded(cx.theme().radius)
                        .bg(cx.theme().muted)
                        .child(
                            Icon::default()
                                .path(icon_path)
                                .small()
                                .text_color(cx.theme().muted_foreground),
                        ),
                )
                .child(
                    v_flex()
                        .min_w_0()
                        .flex_grow(1.0)
                        .gap_0p5()
                        .child(
                            div()
                                .w_full()
                                .text_sm()
                                .text_color(cx.theme().foreground)
                                .truncate()
                                .child(title),
                        )
                        .when_some(subtitle, |this, subtitle| {
                            this.child(
                                div()
                                    .w_full()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .truncate()
                                    .child(subtitle),
                            )
                        }),
                )
                .when(is_selected, |this| {
                    this.child(
                        h_flex()
                            .flex_none()
                            .gap_1()
                            .items_center()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(Self::keycap("↵", cx))
                            .child("Open"),
                    )
                })
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.activate_index(index, window, cx);
                }));
            results = results.child(
                v_flex()
                    .w_full()
                    .flex_none()
                    .when_some(section_label, |this, label| {
                        this.child(
                            div()
                                .h(px(RESULT_SECTION_HEIGHT))
                                .flex()
                                .items_end()
                                .px_3()
                                .pb_1()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(label),
                        )
                    })
                    .child(row),
            );
        }

        v_flex()
            .key_context(COMMAND_PALETTE_CONTEXT)
            .on_action(cx.listener(Self::on_select_next))
            .on_action(cx.listener(Self::on_select_previous))
            .on_action(cx.listener(Self::on_confirm))
            .on_action(cx.listener(Self::on_dismiss))
            .on_action(cx.listener(Self::on_open_palette))
            .w_full()
            .child(
                h_flex()
                    .w_full()
                    .h_12()
                    .items_center()
                    .gap_3()
                    .px_4()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        Icon::default()
                            .path("icons/search.svg")
                            .small()
                            .text_color(cx.theme().muted_foreground),
                    )
                    .child(
                        Input::new(&self.search_input)
                            .w_full()
                            .appearance(false)
                            .border_0()
                            .focus_bordered(false),
                    )
                    .child(Self::keycap("Esc", cx)),
            )
            .when(self.filtered_entries.is_empty(), |this| {
                this.child(
                    v_flex()
                        .w_full()
                        .h(px(144.0))
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .child(
                            div()
                                .size_9()
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .bg(cx.theme().muted)
                                .child(
                                    Icon::default()
                                        .path("icons/search.svg")
                                        .small()
                                        .text_color(cx.theme().muted_foreground),
                                ),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().foreground)
                                .child(format!("No results for “{query}”")),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child("Try a request, folder, or command name"),
                        ),
                )
            })
            .when(!self.filtered_entries.is_empty(), |this| {
                this.child(div().p_2().child(results))
            })
            .child(
                h_flex()
                    .w_full()
                    .h_10()
                    .items_center()
                    .justify_between()
                    .px_4()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        h_flex()
                            .gap_1p5()
                            .items_center()
                            .child(Self::keycap("↑", cx))
                            .child(Self::keycap("↓", cx))
                            .child("Navigate"),
                    )
                    .child(
                        h_flex()
                            .gap_1p5()
                            .items_center()
                            .child(Self::keycap("↵", cx))
                            .child("Open")
                            .child(div().w_2())
                            .child(Self::keycap("Esc", cx))
                            .child("Close"),
                    ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

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
    fn empty_query_limits_recent_requests_without_limiting_search() {
        let entries = (0..12)
            .map(|rank| {
                entry(
                    &format!("Request {rank}"),
                    CommandPaletteEntryKind::Request {
                        request_id: Ulid::new(),
                    },
                    Some(rank),
                )
            })
            .collect::<Vec<_>>();

        let empty_results = filter_command_palette_entries(&entries, "");
        assert_eq!(empty_results.len(), MAX_RECENT_REQUESTS);
        assert_eq!(empty_results[0].title, "Request 0");
        assert_eq!(empty_results[9].title, "Request 9");

        assert_eq!(
            titles(&filter_command_palette_entries(&entries, "Request")),
            (0..12)
                .map(|rank| format!("Request {rank}"))
                .collect::<Vec<_>>()
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

    #[test]
    fn selection_index_resets_after_filtering() {
        let selected_index = 4;
        let entries = vec![entry(
            "Health",
            CommandPaletteEntryKind::Request {
                request_id: Ulid::new(),
            },
            None,
        )];
        let filtered = filter_command_palette_entries(&entries, "health");

        assert_eq!(filtered.len(), 1);
        assert_ne!(selected_index, reset_selection_index());
        assert_eq!(reset_selection_index(), 0);
    }

    #[test]
    fn selection_index_helpers_wrap_and_handle_empty_lists() {
        assert_eq!(next_selection_index(0, 0), 0);
        assert_eq!(previous_selection_index(0, 0), 0);
        assert_eq!(next_selection_index(0, 1), 0);
        assert_eq!(previous_selection_index(0, 1), 0);
        assert_eq!(next_selection_index(2, 3), 0);
        assert_eq!(previous_selection_index(0, 3), 2);
        assert_eq!(next_selection_index(0, 3), 1);
        assert_eq!(previous_selection_index(2, 3), 1);
        assert_eq!(previous_selection_index(8, 3), 2);
    }

    #[test]
    fn times_large_synthetic_entry_construction() {
        const ENTRY_COUNT: usize = 25_000;
        let recent_request_ids = (0..100).map(|_| Ulid::new()).collect::<Vec<_>>();
        let descriptors = (0..ENTRY_COUNT)
            .map(|index| {
                let is_recent = index < recent_request_ids.len();
                WorkspaceTreeDescriptor {
                    id: if is_recent {
                        recent_request_ids[index]
                    } else {
                        Ulid::new()
                    },
                    kind: if index % 10 == 0 {
                        TreeNodeKind::Folder
                    } else {
                        TreeNodeKind::Request
                    },
                    name: format!("Entry {index}"),
                    ancestor_path: vec!["Workspace".to_string(), format!("Group {}", index / 100)],
                }
            })
            .collect();

        let started = Instant::now();
        let entries = build_entries_from_descriptors(descriptors, &recent_request_ids);
        let elapsed = started.elapsed();

        assert_eq!(entries.len(), ENTRY_COUNT + COMMANDS.len());
        eprintln!(
            "built {} command-palette entries in {elapsed:?}",
            entries.len()
        );
    }
}
