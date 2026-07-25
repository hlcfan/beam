use super::super::*;

enum FileState {
    Waiting,
    Importing,
    Done { summary: String },
    Failed { message: String },
}

struct FileRow {
    relative_label: Option<String>,
    detected: DetectedSource,
    state: FileState,
    plan: Option<ImportPlan>,
    command_id: Option<String>,
    imported_workspace_id: Option<Ulid>,
    needs_new_workspace: bool,
}

pub(in crate::ui) struct ImportDialogView {
    files: HashMap<PathBuf, FileRow>,
    importing: bool,
    any_success: bool,
    all_done: bool,
    enqueued_count: usize,
    completed_count: usize,
    cancellation: Arc<AtomicBool>,
    show_cancel_confirm: bool,
    was_cancelled: bool,
    app_command_tx: std::sync::mpsc::SyncSender<AppCommand>,
}

impl ImportDialogView {
    pub(in crate::ui) fn is_importing(&self) -> bool {
        self.importing
    }

    pub(in crate::ui) fn new(
        app_command_tx: std::sync::mpsc::SyncSender<AppCommand>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Self {
        Self {
            files: HashMap::new(),
            importing: false,
            any_success: false,
            all_done: false,
            enqueued_count: 0,
            completed_count: 0,
            cancellation: Arc::new(AtomicBool::new(false)),
            show_cancel_confirm: false,
            was_cancelled: false,
            app_command_tx,
        }
    }

    fn clear_files(&mut self, cx: &mut Context<Self>) {
        self.files.clear();
        self.any_success = false;
        self.all_done = false;
        self.enqueued_count = 0;
        self.completed_count = 0;
        self.show_cancel_confirm = false;
        self.was_cancelled = false;
        cx.notify();
    }

    pub(in crate::ui) fn handle_import_result(
        &mut self,
        result: ImportResult,
        command_id: String,
        cx: &mut Context<Self>,
    ) {
        if self
            .files
            .values()
            .any(|f| f.command_id.as_deref() == Some(&command_id))
        {
            self.completed_count += 1;
            match result {
                ImportResult::Done {
                    summary,
                    counts: _,
                    workspace_id,
                    imported_into_current,
                } => {
                    self.any_success = true;
                    for row in self
                        .files
                        .values_mut()
                        .filter(|f| f.command_id.as_deref() == Some(&command_id))
                    {
                        if !imported_into_current {
                            row.imported_workspace_id = Some(workspace_id);
                        }
                        row.state = FileState::Done {
                            summary: summary.clone(),
                        };
                    }
                }
                ImportResult::Failed {
                    message,
                    partial_workspace_created,
                } => {
                    log::error!("import failed: {message}");
                    let mut msg = message;
                    if partial_workspace_created {
                        msg.push_str(
                            "\nWorkspace created but partially imported — delete it if unwanted.",
                        );
                    }
                    for row in self
                        .files
                        .values_mut()
                        .filter(|f| f.command_id.as_deref() == Some(&command_id))
                    {
                        row.state = FileState::Failed {
                            message: msg.clone(),
                        };
                    }
                }
                ImportResult::Canceled => {
                    for row in self
                        .files
                        .values_mut()
                        .filter(|f| f.command_id.as_deref() == Some(&command_id))
                    {
                        if matches!(row.state, FileState::Importing) {
                            row.state = FileState::Failed {
                                message: "Canceled by user".to_string(),
                            };
                        }
                    }
                }
            }

            if self.completed_count >= self.enqueued_count {
                self.importing = false;
                self.all_done = true;
                self.show_cancel_confirm = false;
                if self.any_success && !self.was_cancelled {
                    if let Some(first_done) = self
                        .files
                        .values()
                        .find(|f| matches!(f.state, FileState::Done { .. }))
                    {
                        if let Some(workspace_id) = first_done.imported_workspace_id {
                            let _ = self.app_command_tx.send(AppCommand::SwitchWorkspace {
                                workspace_id,
                                command_id: next_command_id(),
                            });
                        }
                    }
                }
            }
            cx.notify();
        }
    }

    fn process_paths(
        &mut self,
        paths: Vec<PathBuf>,
        folder_root: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let paths: Vec<PathBuf> = paths
            .into_iter()
            .filter(|path| {
                if self.files.contains_key(path) {
                    return false;
                }
                self.files.insert(
                    path.clone(),
                    FileRow {
                        relative_label: folder_root.as_ref().and_then(|root| {
                            path.strip_prefix(root)
                                .ok()
                                .map(|path| path.to_string_lossy().to_string())
                        }),
                        detected: DetectedSource::Unknown,
                        state: FileState::Waiting,
                        plan: None,
                        command_id: None,
                        imported_workspace_id: None,
                        needs_new_workspace: false,
                    },
                );
                true
            })
            .collect();
        if paths.is_empty() {
            return;
        }
        let view = cx.entity();
        cx.spawn_in(window, async move |_, cx| {
            for path in paths {
                let relative = folder_root
                    .as_ref()
                    .and_then(|root| path.strip_prefix(root).ok().map(|p| p.to_path_buf()));
                let read_result = cx
                    .background_executor()
                    .spawn({
                        let path = path.clone();
                        async move { std::fs::read_to_string(&path).ok() }
                    })
                    .await;
                match read_result {
                    Some(content) => {
                        let ext_hint = path
                            .extension()
                            .and_then(|e| e.to_str())
                            .map(|s| s.to_string());
                        let detection =
                            cx.background_executor()
                                .spawn({
                                    let content = content.clone();
                                    let ext_hint = ext_hint.clone();
                                    async move {
                                        crate::importers::detect(&content, ext_hint.as_deref())
                                    }
                                })
                                .await;
                        if detection == DetectedSource::Unknown {
                            view.update_in(cx, |this, _, cx| {
                                this.files.insert(
                                    path.clone(),
                                    FileRow {
                                        relative_label: relative
                                            .map(|p| p.to_string_lossy().to_string()),
                                        detected: detection,
                                        state: FileState::Failed {
                                            message: "Unknown format".to_string(),
                                        },
                                        plan: None,
                                        command_id: None,
                                        imported_workspace_id: None,
                                        needs_new_workspace: false,
                                    },
                                );
                                cx.notify();
                            })
                            .ok();
                            continue;
                        }
                        let _parser = parser_for(&detection);
                        let plan_result = cx
                            .background_executor()
                            .spawn({
                                let content = content.clone();
                                let detection = detection.clone();
                                async move {
                                    if let Some(parser) = parser_for(&detection) {
                                        // TODO: Insomnia "all data" exports may contain multiple
                                        // workspace resources. `InsomniaParser::parse` returns
                                        // only the first one, so split these exports with
                                        // `list_workspaces`/`for_workspace` before enqueueing.
                                        parser.parse(&content)
                                    } else {
                                        Err(crate::error::BeamError::Validation {
                                            message: "Uknown format".to_string(),
                                        })
                                    }
                                }
                            })
                            .await;
                        match plan_result {
                            Ok(plan) => {
                                let needs_new_workspace =
                                    crate::importers::content_has_workspace(&content);

                                view.update_in(cx, |this, _, cx| {
                                    this.files.insert(
                                        path.clone(),
                                        FileRow {
                                            relative_label: relative
                                                .map(|p| p.to_string_lossy().to_string()),
                                            detected: detection,
                                            state: FileState::Waiting,
                                            plan: Some(plan),
                                            command_id: None,
                                            imported_workspace_id: None,
                                            needs_new_workspace,
                                        },
                                    );
                                    cx.notify();
                                })
                                .ok();
                            }
                            Err(err) => {
                                view.update_in(cx, |this, _, cx| {
                                    this.files.insert(
                                        path.clone(),
                                        FileRow {
                                            relative_label: relative
                                                .map(|p| p.to_string_lossy().to_string()),
                                            detected: detection,
                                            state: FileState::Failed {
                                                message: err.to_string(),
                                            },
                                            plan: None,
                                            command_id: None,
                                            imported_workspace_id: None,
                                            needs_new_workspace: false,
                                        },
                                    );
                                    cx.notify();
                                })
                                .ok();
                            }
                        }
                    }
                    None => {
                        view.update_in(cx, |this, _, cx| {
                            this.files.insert(
                                path.clone(),
                                FileRow {
                                    relative_label: relative
                                        .map(|p| p.to_string_lossy().to_string()),
                                    detected: DetectedSource::Unknown,
                                    state: FileState::Failed {
                                        message: "Could not read file".to_string(),
                                    },
                                    plan: None,
                                    command_id: None,
                                    imported_workspace_id: None,
                                    needs_new_workspace: false,
                                },
                            );
                            cx.notify();
                        })
                        .ok();
                    }
                }
            }
        })
        .detach();
    }

    fn handle_drop_paths(
        &mut self,
        paths: ExternalPaths,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut file_paths: Vec<PathBuf> = Vec::new();
        let mut folder_paths: Vec<PathBuf> = Vec::new();
        for p in paths.0.iter() {
            if p.is_dir() {
                folder_paths.push(p.clone());
            } else if p.is_file() {
                file_paths.push(p.clone());
            }
        }
        if !file_paths.is_empty() {
            self.process_paths(file_paths, None, window, cx);
        }
        for folder in folder_paths {
            let view = cx.entity();
            let root = folder.clone();
            let root_for_scan = root.clone();
            cx.spawn_in(window, async move |_, cx| {
                let scan_result = cx
                    .background_executor()
                    .spawn(async move { scanner::scan_folder(&root_for_scan) })
                    .await;
                match scan_result {
                    Ok(scanned_paths) => {
                        view.update_in(cx, |this, window, cx| {
                            this.process_paths(scanned_paths, Some(root.clone()), window, cx);
                        })
                        .ok();
                    }
                    Err(err) => {
                        view.update_in(cx, |this, _, cx| {
                            this.files.insert(
                                root.clone(),
                                FileRow {
                                    relative_label: None,
                                    detected: DetectedSource::Unknown,
                                    state: FileState::Failed {
                                        message: format!("Folder scan error: {err:?}"),
                                    },
                                    plan: None,
                                    command_id: None,
                                    imported_workspace_id: None,
                                    needs_new_workspace: false,
                                },
                            );
                            cx.notify();
                        })
                        .ok();
                    }
                }
            })
            .detach();
        }
    }

    fn render_file_row(
        &self,
        path: &PathBuf,
        row: &FileRow,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");
        let label = if let Some(ref rel) = row.relative_label {
            rel.to_string()
        } else {
            file_name.to_string()
        };
        let tag_text = tag_label(&row.detected);
        let tag = Tag::secondary().small().outline().child(tag_text);
        let status = match &row.state {
            FileState::Waiting => h_flex()
                .items_center()
                .gap_1()
                .child(
                    div()
                        .size(px(6.0))
                        .rounded_full()
                        .bg(cx.theme().muted_foreground),
                )
                .child("Waiting")
                .text_xs()
                .text_color(cx.theme().muted_foreground),
            FileState::Importing => div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Importing..."),
            FileState::Done { summary } => {
                let msg = summary.clone();
                h_flex().items_center().child(
                    div()
                        .id("import-done-icon")
                        .tooltip(move |window, cx| Tooltip::new(msg.clone()).build(window, cx))
                        .child(
                            Icon::default()
                                .path("icons/check.svg")
                                .size(px(14.0))
                                .text_color(cx.theme().success),
                        ),
                )
            }
            FileState::Failed { message } => {
                let msg = message.clone();
                h_flex().items_center().child(
                    div()
                        .id("import-fail-icon")
                        .tooltip(move |window, cx| Tooltip::new(msg.clone()).build(window, cx))
                        .child(
                            Icon::default()
                                .path("icons/info.svg")
                                .size(px(14.0))
                                .text_color(cx.theme().muted_foreground),
                        ),
                )
            }
        };
        v_flex().w_full().gap_0().child(
            h_flex()
                .w_full()
                .items_center()
                .justify_between()
                .py_1()
                .rounded(px(4.0))
                .child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .flex_1()
                        .child(div().text_sm().truncate().child(label)),
                )
                .child(h_flex().items_center().gap_2().child(tag).child(status)),
        )
    }
}

impl Render for ImportDialogView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let has_files = !self.files.is_empty();
        let view = cx.entity();

        v_flex()
            .w_full()
            .px_2()
            .gap_4()
            .when(!has_files, |this| {
                this.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(
                            "Supports Postman collections and environments, and Insomnia exports. Files are imported as one batch: an included Insomnia workspace creates a new workspace; otherwise, everything goes into the active workspace.",
                        ),
                )
                .child({
                    let _drop_view = view.clone();
                    div()
                        .w_full()
                        .h(px(160.0))
                        .rounded(px(8.0))
                        .border_dashed()
                        .border_1()
                        .border_color(cx.theme().border)
                        .bg(cx.theme().background)
                        .cursor_pointer()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap_3()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |_this, _: &MouseDownEvent, window, cx| {
                                let rx = cx.prompt_for_paths(PathPromptOptions {
                                    files: true,
                                    directories: true,
                                    multiple: true,
                                    prompt: None,
                                });
                                let entity = cx.entity();
                                cx.spawn_in(window, async move |_, cx| {
                                    let picked = match rx.await {
                                        Ok(Ok(Some(p))) => p,
                                        _ => return,
                                    };
                                    entity
                                        .update_in(cx, move |this, window, cx| {
                                            this.handle_drop_paths(
                                                ExternalPaths(picked.into()),
                                                window,
                                                cx,
                                            );
                                        })
                                        .ok();
                                })
                                .detach();
                            }),
                        )
                        .drag_over::<ExternalPaths>(|style, _, _, cx| {
                            style
                                .bg(cx.theme().accent.opacity(0.08))
                                .border_color(cx.theme().accent)
                        })
                        .on_drop(cx.listener(move |this, paths: &ExternalPaths, window, cx| {
                            this.handle_drop_paths(paths.clone(), window, cx);
                        }))
                        .child(
                            Icon::default()
                                .path("icons/upload.svg")
                                .size(px(28.0))
                                .text_color(cx.theme().muted_foreground),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child("Drag files or folders here, or click to choose."),
                        )
                })
            })
            .when(has_files, |this| {
                this.child(
                    h_flex()
                        .w_full()
                        .justify_between()
                        .items_center()
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!("{} files", self.files.len())),
                        )
                        .child({
                            h_flex()
                                .text_xs()
                                .cursor_pointer()
                                .text_color(cx.theme().muted_foreground)
                                .when(self.importing, |this| this.opacity(0.4).cursor_default())
                                .child("Clear")
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _: &MouseDownEvent, _window, cx| {
                                        if !this.importing {
                                            this.clear_files(cx);
                                        }
                                    }),
                                )
                        }),
                )
            })
            .when(has_files, |this| {
                let all_failed = self
                    .files
                    .values()
                    .all(|f| matches!(f.state, FileState::Failed { .. }));
                this.child(
                    v_flex()
                        .w_full()
                        .max_h(px(320.0))
                        .overflow_y_scrollbar()
                        .gap_0()
                        .when(all_failed, |this| {
                            this.child(
                                div()
                                    .w_full()
                                    .px_2()
                                    .py_1()
                                    .mb_1()
                                    .rounded(px(4.0))
                                    .bg(cx.theme().danger.opacity(0.08))
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().danger)
                                            .child(
                                                "No files could be imported. See details below.",
                                            ),
                                    ),
                            )
                        })
                        .children({
                            let mut children = Vec::new();
                            for (path, row) in &self.files {
                                children.push(self.render_file_row(path, row, cx));
                            }
                            children
                        }),
                )
            })
            .child({
                if self.show_cancel_confirm && self.importing {
                    v_flex()
                        .w_full()
                        .gap_3()
                        .pt_2()
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(
                                    "An import is in progress. Canceling will stop the \
                                     current import. Are you sure?",
                                ),
                        )
                        .child(
                            h_flex()
                                .w_full()
                                .justify_between()
                                .child({
                                    let keep_btn = Button::new("import-dialog-keep-importing")
                                        .ghost()
                                        .small()
                                        .cursor_pointer()
                                        .label("Keep importing");
                                    keep_btn.on_click(cx.listener(
                                        |this, _: &ClickEvent, _window, cx| {
                                            this.show_cancel_confirm = false;
                                            cx.notify();
                                        },
                                    ))
                                })
                                .child(
                                    Button::new("import-dialog-cancel-import")
                                        .primary()
                                        .small()
                                        .cursor_pointer()
                                        .label("Cancel import")
                                        .on_click(cx.listener(
                                            |this, _: &ClickEvent, _window, cx| {
                                                this.cancellation.store(true, Ordering::SeqCst);
                                                this.was_cancelled = true;
                                                for row in this.files.values_mut() {
                                                    match row.state {
                                                        FileState::Importing => {
                                                            row.state = FileState::Failed {
                                                                message: "Canceled by user"
                                                                    .to_string(),
                                                            };
                                                        }
                                                        FileState::Waiting => {
                                                            row.state = FileState::Failed {
                                                                message: "Canceled before import"
                                                                    .to_string(),
                                                            };
                                                        }
                                                        _ => {}
                                                    }
                                                }
                                                this.show_cancel_confirm = false;
                                                cx.notify();
                                            },
                                        )),
                                ),
                        )
                        .into_any_element()
                } else {
                    h_flex()
                        .w_full()
                        .justify_between()
                        .pt_2()
                        .child({
                            let mut cancel_btn = Button::new("import-dialog-cancel")
                                .ghost()
                                .small()
                                .cursor_pointer()
                                .label("Cancel");
                            if self.importing {
                                cancel_btn = cancel_btn.tooltip("Import in progress");
                            }
                            cancel_btn.on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                if this.importing {
                                    this.show_cancel_confirm = true;
                                    cx.notify();
                                } else {
                                    window.close_dialog(cx);
                                }
                            }))
                        })
                        .child(
                            Button::new("import-dialog-submit")
                                .primary()
                                .small()
                                .cursor_pointer()
                                .label(if self.all_done {
                                    "Done"
                                } else if self.importing {
                                    "Importing..."
                                } else {
                                    "Import"
                                })
                                .disabled(self.files.is_empty() || self.importing)
                                .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                                    if this.all_done {
                                        _window.close_dialog(cx);
                                        return;
                                    }
                                    if this.importing {
                                        return;
                                    }
                                    let has_waiting = this.files.values().any(|f| {
                                        f.plan.is_some() && matches!(f.state, FileState::Waiting)
                                    });
                                    if !has_waiting {
                                        return;
                                    }
                                    this.cancellation = Arc::new(AtomicBool::new(false));
                                    this.was_cancelled = false;
                                    this.show_cancel_confirm = false;
                                    let workspace_count = this
                                        .files
                                        .values()
                                        .filter(|row| {
                                            row.plan.is_some()
                                                && matches!(row.state, FileState::Waiting)
                                                && row.needs_new_workspace
                                        })
                                        .count();
                                    let has_separate_environment = this.files.values().any(|row| {
                                        row.plan.is_some()
                                            && matches!(row.state, FileState::Waiting)
                                            && matches!(
                                                row.detected,
                                                DetectedSource::PostmanEnvironment
                                            )
                                    });
                                    if workspace_count > 1 && has_separate_environment {
                                        for row in this.files.values_mut() {
                                            if row.plan.is_some()
                                                && matches!(row.state, FileState::Waiting)
                                            {
                                                row.state = FileState::Failed {
                                                    message: "An import batch with multiple workspaces cannot include separate environment files"
                                                        .to_string(),
                                                };
                                            }
                                        }
                                        this.all_done = true;
                                        cx.notify();
                                        return;
                                    }

                                    let mut queued_plans = this
                                        .files
                                        .iter()
                                        .filter(|row| {
                                            row.1.plan.is_some()
                                                && matches!(row.1.state, FileState::Waiting)
                                        })
                                        .filter_map(|(path, row)| {
                                            row.plan
                                                .clone()
                                                .map(|plan| {
                                                    (
                                                        path.clone(),
                                                        plan,
                                                        row.needs_new_workspace,
                                                    )
                                                })
                                        })
                                        .collect::<Vec<_>>();
                                    queued_plans.sort_by(|a, b| a.0.cmp(&b.0));
                                    let Some((batch_plan, needs_new_workspace)) =
                                        crate::importers::merge_file_plans(
                                            queued_plans
                                                .into_iter()
                                                .map(|(_, plan, needs_workspace)| {
                                                    (plan, needs_workspace)
                                                })
                                                .collect(),
                                        )
                                    else {
                                        return;
                                    };

                                    let command_id = next_command_id();
                                    let job = ImportJob {
                                        plan: batch_plan,
                                        cancellation: this.cancellation.clone(),
                                        needs_new_workspace,
                                    };
                                    let _ = this.app_command_tx.send(AppCommand::RunImport {
                                        job,
                                        command_id: command_id.clone(),
                                    });
                                    for row in this.files.values_mut() {
                                        if row.plan.is_some()
                                            && matches!(row.state, FileState::Waiting)
                                        {
                                            row.state = FileState::Importing;
                                            row.command_id = Some(command_id.clone());
                                        }
                                    }
                                    this.enqueued_count = 1;
                                    this.completed_count = 0;
                                    this.importing = true;
                                    cx.notify();
                                })),
                        )
                        .into_any_element()
                }
            })
    }
}
