mod command_palette;
mod environment;
mod import;
mod key_bindings;
mod settings;
mod tree;
mod workspace;

pub(super) use command_palette::{
    CommandPaletteDialogView, ConfirmPaletteItem, DismissCommandPalette, SelectNextPaletteItem,
    SelectPreviousPaletteItem,
};
pub(super) use environment::EnvironmentManagerDialogView;
pub(super) use import::ImportDialogView;
pub(super) use key_bindings::KeyBindingsDialogView;
pub(super) use settings::SettingsDialogView;
pub(super) use tree::{TreeNodeDeleteDialogView, TreeRenameDialogView};
pub(super) use workspace::{
    WorkspaceDeleteDialogView, WorkspaceDialogMode, WorkspaceNameDialogView,
};

use super::*;

impl BeamView {
    pub(in crate::ui) fn open_command_palette(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.command_palette_dialog_view.is_some() {
            return;
        }
        let entries = command_palette::build_command_palette_entries(
            &self.shell.workspace_tree,
            self.request_view_histories.recent_request_ids(),
        );
        let beam_view = cx.entity();
        let palette_view =
            cx.new(|cx| CommandPaletteDialogView::new(beam_view.clone(), entries, window, cx));
        self.command_palette_dialog_view = Some(palette_view.clone());
        let beam_view_for_close = beam_view.clone();

        cx.defer(move |cx| {
            if let Some(root_window) = cx.active_window().and_then(|w| w.downcast::<Root>()) {
                let _ = root_window.update(cx, |_, window, cx| {
                    window.defer(cx, move |window, cx| {
                        let palette_view_for_focus = palette_view.clone();
                        window.open_dialog(cx, move |dialog, _, _| {
                            dialog
                                .w(px(640.0))
                                .p_0()
                                .child(palette_view.clone())
                                .close_button(false)
                                .on_close({
                                    let beam_view = beam_view_for_close.clone();
                                    move |_, _, cx| {
                                        beam_view.update(cx, |beam_view, cx| {
                                            beam_view.command_palette_dialog_view = None;
                                            cx.notify();
                                        });
                                    }
                                })
                        });
                        window.defer(cx, move |window, cx| {
                            palette_view_for_focus.update(cx, |palette, cx| {
                                palette.focus_search_input(window, cx);
                            });
                        });
                    });
                });
            }
        });
        cx.notify();
    }

    pub(in crate::ui) fn open_settings_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let beam_view = cx.entity();
        let settings_view = cx.new(|cx| SettingsDialogView::new(beam_view.clone(), window, cx));
        self.settings_dialog_view = Some(settings_view.clone());
        cx.defer(move |cx| {
            if let Some(root_window) = cx.active_window().and_then(|w| w.downcast::<Root>()) {
                let _ = root_window.update(cx, |_, window, cx| {
                    window.defer(cx, move |window, cx| {
                        window.open_dialog(cx, move |dialog, _, _| {
                            dialog
                                .title("Settings")
                                .w(px(920.0))
                                .max_w(px(1200.0))
                                .child(settings_view.clone())
                        });
                    });
                });
            }
        });
        cx.notify();
    }

    pub(in crate::ui) fn open_import_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let import_view =
            cx.new(|cx| ImportDialogView::new(self.app_command_tx.clone(), window, cx));
        self.import_dialog_view = Some(import_view.clone());
        cx.defer(move |cx| {
            if let Some(root_window) = cx.active_window().and_then(|w| w.downcast::<Root>()) {
                let _ = root_window.update(cx, |_, window, cx| {
                    window.defer(cx, move |window, cx| {
                        window.open_dialog(cx, move |dialog, _, _| {
                            dialog
                                .title("Import requests")
                                .w(px(640.0))
                                .child(import_view.clone())
                                .keyboard(false)
                                .overlay_closable(false)
                        });
                    });
                });
            }
        });
        cx.notify();
    }

    pub(in crate::ui) fn open_key_bindings_dialog(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = cx.new(|_cx| KeyBindingsDialogView);
        self.key_bindings_dialog_view = Some(view.clone());
        cx.defer(move |cx| {
            if let Some(root_window) = cx.active_window().and_then(|w| w.downcast::<Root>()) {
                let _ = root_window.update(cx, |_, window, cx| {
                    window.defer(cx, move |window, cx| {
                        window.open_dialog(cx, move |dialog, _, _| {
                            dialog
                                .title("Key Bindings")
                                .w(px(520.0))
                                .child(view.clone())
                        });
                    });
                });
            }
        });
        cx.notify();
    }

    pub(in crate::ui) fn show_create_workspace_dialog(&mut self, cx: &mut Context<Self>) {
        self.open_workspace_name_dialog(WorkspaceDialogMode::Create, String::new(), cx);
    }

    pub(in crate::ui) fn show_rename_workspace_dialog(&mut self, cx: &mut Context<Self>) {
        let current_name = self.shell.workspace.workspace_name.clone();
        self.open_workspace_name_dialog(WorkspaceDialogMode::Rename, current_name, cx);
    }

    pub(in crate::ui) fn show_delete_tree_node_dialog(
        &mut self,
        node_id: Ulid,
        node_kind: TreeNodeKind,
        cx: &mut Context<Self>,
    ) {
        let Some(node) = self.shell.workspace_tree.node(node_id) else {
            return;
        };
        let node_name = node.name.clone();
        let view = cx.entity();
        cx.defer(move |cx| {
            if let Some(root_window) = cx.active_window().and_then(|w| w.downcast::<Root>()) {
                let _ = root_window.update(cx, |_, window, cx| {
                    let dialog_view = cx.new(|cx| {
                        TreeNodeDeleteDialogView::new(
                            view.clone(),
                            node_id,
                            node_kind,
                            node_name.clone(),
                            window,
                            cx,
                        )
                    });
                    let submit_view = dialog_view.clone();
                    window.defer(cx, move |window, cx| {
                        window.open_dialog(cx, move |dialog, _, _| {
                            let ok_view = submit_view.clone();
                            let title = match node_kind {
                                TreeNodeKind::Folder => "Delete Folder",
                                TreeNodeKind::Request => "Delete Request",
                            };
                            dialog
                                .title(title)
                                .w(px(500.0))
                                .child(dialog_view.clone())
                                .on_ok(move |_, window, cx| {
                                    let _ = ok_view.update(cx, |this, cx| {
                                        this.submit(window, cx);
                                    });
                                    false
                                })
                        });
                    });
                });
            }
        });
        cx.notify();
    }

    pub(in crate::ui) fn show_delete_workspace_dialog(&mut self, cx: &mut Context<Self>) {
        let Some(workspace_id) = self.shell.workspace.workspace_id else {
            return;
        };

        let workspace_name = self.shell.workspace.workspace_name.clone();
        let view = cx.entity();
        cx.defer(move |cx| {
            if let Some(root_window) = cx.active_window().and_then(|w| w.downcast::<Root>()) {
                let _ = root_window.update(cx, |_, window, cx| {
                    let dialog_view = cx.new(|cx| {
                        WorkspaceDeleteDialogView::new(
                            view.clone(),
                            workspace_id,
                            workspace_name.clone(),
                            window,
                            cx,
                        )
                    });
                    window.defer(cx, move |window, cx| {
                        let submit_view = dialog_view.clone();
                        window.open_dialog(cx, move |dialog, _, _| {
                            let ok_view = submit_view.clone();
                            dialog
                                .title("Delete Workspace")
                                .w(px(500.0))
                                .child(dialog_view.clone())
                                .on_ok(move |_, window, cx| {
                                    let _ = ok_view.update(cx, |this, cx| {
                                        this.submit(window, cx);
                                    });
                                    false
                                })
                        });
                    });
                });
            }
        });
        cx.notify();
    }

    fn open_workspace_name_dialog(
        &mut self,
        mode: WorkspaceDialogMode,
        initial_name: String,
        cx: &mut Context<Self>,
    ) {
        let title = match mode {
            WorkspaceDialogMode::Create => "New Workspace",
            WorkspaceDialogMode::Rename => "Rename Workspace",
        };
        let view = cx.entity();
        cx.defer(move |cx| {
            if let Some(root_window) = cx.active_window().and_then(|w| w.downcast::<Root>()) {
                let _ = root_window.update(cx, |_, window, cx| {
                    let dialog_view = cx.new(|cx| {
                        WorkspaceNameDialogView::new(view.clone(), mode, initial_name, window, cx)
                    });
                    let focus_view = dialog_view.clone();
                    window.defer(cx, move |window, cx| {
                        let submit_view = dialog_view.clone();
                        window.open_dialog(cx, move |dialog, _, _| {
                            let ok_view = submit_view.clone();
                            dialog
                                .title(title)
                                .w(px(460.0))
                                .child(dialog_view.clone())
                                .on_ok(move |_, window, cx| {
                                    let _ = ok_view.update(cx, |this, cx| {
                                        this.submit(window, cx);
                                    });
                                    false
                                })
                        });
                        window.defer(cx, move |window, cx| {
                            let _ = focus_view.update(cx, |this, cx| {
                                this.focus_name_input(window, cx);
                            });
                        });
                    });
                });
            }
        });
        cx.notify();
    }
}
