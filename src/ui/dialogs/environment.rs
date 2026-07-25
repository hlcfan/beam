use super::super::*;

pub(in crate::ui) struct EnvironmentManagerDialogView {
    beam_view: Entity<BeamView>,
    workspace_paths: BeamPaths,
    options: Vec<(Ulid, String)>,
    environment_file_names: HashMap<Ulid, String>,
    selected_id: Option<Ulid>,
    active_environment_id: Option<Ulid>,
    show_environment_selector: bool,
    variables: Vec<EnvironmentVariable>,
    environment_name_input: Entity<InputState>,
    variable_name_inputs: Vec<Entity<InputState>>,
    variable_value_inputs: Vec<Entity<InputState>>,
    variable_input_subscriptions: Vec<Subscription>,
    pending_variables_save_due_at: Option<Instant>,
    variables_save_tick_scheduled: bool,
    variables_save_in_flight: bool,
    suppress_environment_name_change_events: bool,
    environment_name_input_subscription: Option<Subscription>,
    loaded_environment_name: Option<String>,
    pending_new_environment_command_id: Option<String>,
    error: Option<String>,
}

impl EnvironmentManagerDialogView {
    fn sync_environment_options(
        &mut self,
        next_options: Vec<(Ulid, String)>,
        next_file_names: HashMap<Ulid, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let options_changed = self.options != next_options;
        let file_names_changed = self.environment_file_names != next_file_names;
        if options_changed {
            self.options = next_options;
        }
        if file_names_changed {
            self.environment_file_names = next_file_names;
        }

        let selected_exists = self
            .selected_id
            .is_some_and(|id| self.options.iter().any(|(option_id, _)| *option_id == id));
        if selected_exists {
            return options_changed || file_names_changed;
        }

        let previous_selection = self.selected_id;
        self.selected_id = self.options.first().map(|(id, _)| *id);
        if let Some(environment_id) = self.selected_id {
            self.load_variables(environment_id, window, cx);
            return true;
        }

        if previous_selection.is_none() && !options_changed && !file_names_changed {
            return false;
        }

        self.variables.clear();
        self.clear_variable_inputs();
        self.loaded_environment_name = None;
        self.suppress_environment_name_change_events = true;
        self.environment_name_input.update(cx, |input, cx| {
            input.set_value(String::new(), window, cx);
        });
        self.suppress_environment_name_change_events = false;
        self.error = Some("No environment available to manage.".to_string());
        true
    }

    pub(in crate::ui) fn parse_environment_file(content: &str) -> Result<EnvironmentFile, String> {
        toml::from_str::<EnvironmentFile>(content)
            .map_err(|error| format!("Failed to parse environment file: {error}"))
    }

    pub(in crate::ui) fn new(
        beam_view: Entity<BeamView>,
        workspace_paths: BeamPaths,
        options: Vec<(Ulid, String)>,
        environment_file_names: HashMap<Ulid, String>,
        selected_id: Option<Ulid>,
        active_environment_id: Option<Ulid>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let environment_name_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Environment name"));
        let mut view = Self {
            beam_view,
            workspace_paths,
            options,
            environment_file_names,
            selected_id,
            active_environment_id,
            show_environment_selector: true,
            variables: Vec::new(),
            environment_name_input,
            variable_name_inputs: Vec::new(),
            variable_value_inputs: Vec::new(),
            variable_input_subscriptions: Vec::new(),
            pending_variables_save_due_at: None,
            variables_save_tick_scheduled: false,
            variables_save_in_flight: false,
            suppress_environment_name_change_events: false,
            environment_name_input_subscription: None,
            loaded_environment_name: None,
            pending_new_environment_command_id: None,
            error: None,
        };
        let environment_name_input_handle = view.environment_name_input.clone();
        view.environment_name_input_subscription = Some(cx.subscribe_in(
            &view.environment_name_input,
            window,
            move |this, _, ev: &InputEvent, _, cx| {
                if !matches!(ev, InputEvent::Change) {
                    return;
                }
                if this.suppress_environment_name_change_events || this.selected_id.is_none() {
                    return;
                }
                let _ = environment_name_input_handle.read(cx);
                this.error = None;
                this.schedule_variables_save(cx);
            },
        ));
        if let Some(environment_id) = view.selected_id {
            view.load_variables(environment_id, window, cx);
        } else {
            view.error = Some("No environment available to manage.".to_string());
        }
        view
    }

    pub(in crate::ui) fn new_for_sheet(
        beam_view: Entity<BeamView>,
        workspace_paths: BeamPaths,
        selected_option: Option<(Ulid, String, String)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (options, environment_file_names, selected_id) =
            if let Some((environment_id, label, file_name)) = selected_option {
                (
                    vec![(environment_id, label)],
                    HashMap::from([(environment_id, file_name)]),
                    Some(environment_id),
                )
            } else {
                (Vec::new(), HashMap::new(), None)
            };
        let mut view = Self::new(
            beam_view,
            workspace_paths,
            options,
            environment_file_names,
            selected_id,
            selected_id,
            window,
            cx,
        );
        view.show_environment_selector = false;
        view
    }

    fn environment_file_path(&self, environment_id: Ulid) -> Option<PathBuf> {
        let file_name = self.environment_file_names.get(&environment_id)?;
        environment_file_path_for_workspace(&self.workspace_paths, file_name)
    }

    fn load_variables(
        &mut self,
        environment_id: Ulid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = self.environment_file_path(environment_id) else {
            self.variables.clear();
            self.clear_variable_inputs();
            self.error = Some("Environment file not found.".to_string());
            return;
        };
        let content = match fs::read_to_string(&path) {
            Ok(content) => content,
            Err(error) => {
                self.variables.clear();
                self.clear_variable_inputs();
                self.error = Some(format!("Failed to read environment file: {error}"));
                return;
            }
        };
        let parsed = match Self::parse_environment_file(&content) {
            Ok(parsed) => parsed,
            Err(error) => {
                self.variables.clear();
                self.clear_variable_inputs();
                self.error = Some(error);
                return;
            }
        };
        let environment_name = parsed.environment.name.clone();
        self.variables = parsed.variables;
        self.loaded_environment_name = Some(environment_name.clone());
        self.rebuild_variable_inputs(window, cx);
        self.suppress_environment_name_change_events = true;
        self.environment_name_input.update(cx, |input, cx| {
            input.set_value(environment_name.clone(), window, cx);
        });
        self.suppress_environment_name_change_events = false;
        if let Some((_, label)) = self
            .options
            .iter_mut()
            .find(|(option_id, _)| *option_id == environment_id)
        {
            *label =
                Self::environment_option_label(&parsed.environment.name, parsed.environment.scope);
        }
        self.error = None;
    }

    fn next_default_environment_name(&self) -> String {
        let base_name = "New Environment";
        if !self
            .options
            .iter()
            .any(|(_, label)| label.eq_ignore_ascii_case(base_name))
        {
            return base_name.to_string();
        }
        let mut suffix = 2_u32;
        loop {
            let candidate = format!("{base_name} {suffix}");
            if !self
                .options
                .iter()
                .any(|(_, label)| label.eq_ignore_ascii_case(&candidate))
            {
                return candidate;
            }
            suffix += 1;
        }
    }

    fn add_environment(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let environment_name = self.next_default_environment_name();
        let command_id = next_command_id();
        let command = AppCommand::CreateEnvironment {
            name: environment_name,
            command_id: command_id.clone(),
        };
        let send_result = self
            .beam_view
            .update(cx, move |this, _| this.publish_app_command(command));
        match send_result {
            Ok(()) => {
                self.pending_new_environment_command_id = Some(command_id);
                self.error = None;
            }
            Err(error) => {
                self.pending_new_environment_command_id = None;
                self.error = Some(format!("Failed to queue environment creation: {error}"));
            }
        }
        cx.notify();
    }

    fn focus_environment_name_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.environment_name_input.update(cx, |input, cx| {
            input.focus(window, cx);
            let cursor_end = input.value().encode_utf16().count() as u32;
            input.set_cursor_position(Position::new(0, cursor_end), window, cx);
        });
    }

    fn delete_selected_environment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(environment_id) = self.selected_id else {
            self.error = Some("No environment selected.".to_string());
            cx.notify();
            return;
        };
        let command = AppCommand::DeleteEnvironment {
            environment_id,
            command_id: next_command_id(),
        };
        let send_result = self
            .beam_view
            .update(cx, move |this, _| this.publish_app_command(command));
        if let Err(error) = send_result {
            self.error = Some(format!("Failed to queue environment deletion: {error}"));
            cx.notify();
            return;
        }

        self.options
            .retain(|(option_environment_id, _)| *option_environment_id != environment_id);
        self.environment_file_names.remove(&environment_id);
        self.pending_variables_save_due_at = None;
        self.variables_save_tick_scheduled = false;
        self.variables_save_in_flight = false;
        self.selected_id = self.options.first().map(|(id, _)| *id);

        if let Some(next_environment_id) = self.selected_id {
            self.load_variables(next_environment_id, window, cx);
        } else {
            self.variables.clear();
            self.clear_variable_inputs();
            self.loaded_environment_name = None;
            self.suppress_environment_name_change_events = true;
            self.environment_name_input.update(cx, |input, cx| {
                input.set_value(String::new(), window, cx);
            });
            self.suppress_environment_name_change_events = false;
            self.error = Some("No environment available to manage.".to_string());
        }
        cx.notify();
    }

    fn environment_option_label(name: &str, _scope: EnvironmentScope) -> String {
        name.to_string()
    }

    fn clear_variable_inputs(&mut self) {
        self.variable_name_inputs.clear();
        self.variable_value_inputs.clear();
        self.variable_input_subscriptions.clear();
    }

    fn rebuild_variable_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_variable_inputs();
        for index in 0..self.variables.len() {
            let key_value = self.variables[index].name.clone();
            let value_value = self.variables[index].value.clone();

            let key_input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Key")
                    .default_value(key_value)
            });
            let value_input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Value")
                    .default_value(value_value)
            });

            let key_input_handle = key_input.clone();
            let key_subscription = cx.subscribe_in(
                &key_input,
                window,
                move |this, _, ev: &InputEvent, _, cx| {
                    if !matches!(ev, InputEvent::Change) {
                        return;
                    }
                    if let Some(variable) = this.variables.get_mut(index) {
                        variable.name = key_input_handle.read(cx).value().to_string();
                        this.schedule_variables_save(cx);
                    }
                },
            );

            let value_input_handle = value_input.clone();
            let value_subscription = cx.subscribe_in(
                &value_input,
                window,
                move |this, _, ev: &InputEvent, _, cx| {
                    if !matches!(ev, InputEvent::Change) {
                        return;
                    }
                    if let Some(variable) = this.variables.get_mut(index) {
                        variable.value = value_input_handle.read(cx).value().to_string();
                        this.schedule_variables_save(cx);
                    }
                },
            );

            self.variable_name_inputs.push(key_input);
            self.variable_value_inputs.push(value_input);
            self.variable_input_subscriptions.push(key_subscription);
            self.variable_input_subscriptions.push(value_subscription);
        }
    }

    fn schedule_variables_save_with_delay(&mut self, delay: Duration, cx: &mut Context<Self>) {
        if self.selected_id.is_none() {
            return;
        }
        self.pending_variables_save_due_at = Some(Instant::now() + delay);
        if self.variables_save_tick_scheduled {
            return;
        }
        self.variables_save_tick_scheduled = true;
        self.schedule_variables_save_tick(cx);
    }

    fn schedule_variables_save(&mut self, cx: &mut Context<Self>) {
        self.schedule_variables_save_with_delay(Duration::from_millis(350), cx);
    }

    fn schedule_variables_save_tick(&self, cx: &mut Context<Self>) {
        let view = cx.entity();
        cx.spawn(async move |_, cx| {
            cx.background_executor()
                .spawn(async move {
                    std::thread::sleep(Duration::from_millis(25));
                })
                .await;
            let _ = view.update(cx, |this, cx| {
                this.process_pending_variables_save(cx);
            });
        })
        .detach();
    }

    fn process_pending_variables_save(&mut self, cx: &mut Context<Self>) {
        if self.variables_save_in_flight {
            self.variables_save_tick_scheduled = false;
            return;
        }
        let Some(due_at) = self.pending_variables_save_due_at else {
            self.variables_save_tick_scheduled = false;
            return;
        };
        if Instant::now() < due_at {
            self.schedule_variables_save_tick(cx);
            return;
        }
        self.pending_variables_save_due_at = None;
        self.variables_save_tick_scheduled = false;
        let Some(environment_id) = self.selected_id else {
            return;
        };
        let updated_name = self
            .environment_name_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        if updated_name.is_empty() {
            self.error = Some("Environment name cannot be empty.".to_string());
            cx.notify();
            return;
        }
        let variables: Vec<EnvironmentVariable> = self
            .variables
            .iter()
            .filter(|variable| !variable.name.trim().is_empty())
            .cloned()
            .collect();
        if self
            .loaded_environment_name
            .as_deref()
            .is_some_and(|name| name != updated_name.as_str())
        {
            let rename_command = AppCommand::RenameEnvironment {
                environment_id,
                new_name: updated_name.clone(),
                command_id: next_command_id(),
            };
            let rename_result = self
                .beam_view
                .update(cx, move |this, _| this.publish_app_command(rename_command));
            if let Err(error) = rename_result {
                self.variables_save_in_flight = false;
                self.error = Some(format!("Failed to queue environment rename: {error}"));
                cx.notify();
                return;
            }
            self.loaded_environment_name = Some(updated_name.clone());
        }
        let command = AppCommand::UpdateEnvironmentVariables {
            environment_id,
            variables,
            command_id: next_command_id(),
        };
        let send_result = self
            .beam_view
            .update(cx, move |this, _| this.publish_app_command(command));
        self.variables_save_in_flight = false;
        self.error = send_result.err().map(|error| {
            if error.starts_with("Backpressure:") {
                self.pending_variables_save_due_at =
                    Some(Instant::now() + Duration::from_millis(100));
            }
            format!("Failed to queue environment save: {error}")
        });
        if self.pending_variables_save_due_at.is_some() && !self.variables_save_tick_scheduled {
            self.variables_save_tick_scheduled = true;
            self.schedule_variables_save_tick(cx);
        }
        cx.notify();
    }

    fn add_variable(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.variables.push(EnvironmentVariable {
            name: String::new(),
            value: String::new(),
            enabled: true,
            description: None,
        });
        self.rebuild_variable_inputs(window, cx);
        self.schedule_variables_save(cx);
        cx.notify();
    }

    fn remove_variable(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.variables.len() {
            return;
        }
        self.variables.remove(index);
        self.rebuild_variable_inputs(window, cx);
        self.schedule_variables_save(cx);
        cx.notify();
    }

    pub(in crate::ui) fn refresh_from_snapshot(
        &mut self,
        workspace_paths: BeamPaths,
        options: Vec<(Ulid, String)>,
        environment_file_names: HashMap<Ulid, String>,
        active_environment_id: Option<Ulid>,
        latest_upsert: Option<(Ulid, String)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workspace_paths = workspace_paths;
        let active_environment_changed = self.active_environment_id != active_environment_id;
        self.active_environment_id = active_environment_id;
        let mut should_notify =
            self.sync_environment_options(options, environment_file_names, window, cx);
        if let Some((environment_id, command_id)) = latest_upsert {
            if self.pending_new_environment_command_id.as_deref() == Some(command_id.as_str())
                && self
                    .options
                    .iter()
                    .any(|(option_id, _)| *option_id == environment_id)
            {
                self.pending_new_environment_command_id = None;
                self.selected_id = Some(environment_id);
                self.load_variables(environment_id, window, cx);
                self.focus_environment_name_input(window, cx);
                should_notify = true;
            }
        }
        if should_notify {
            cx.notify();
        } else if active_environment_changed {
            cx.notify();
        }
    }
}

impl Render for EnvironmentManagerDialogView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let environment_name_has_selection = !self
            .environment_name_input
            .read(cx)
            .selected_range()
            .is_empty();
        let has_selected_environment = self.selected_id.is_some();
        let selected_label = self.selected_id.and_then(|id| {
            self.options
                .iter()
                .find(|(environment_id, _)| *environment_id == id)
                .map(|(_, label)| label.clone())
        });
        let mut variables_panel = v_flex().h_full().w_full().gap_3();
        variables_panel =
            variables_panel.child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .child(div().text_sm().font_semibold().child(
                        selected_label.unwrap_or_else(|| "No environment selected".to_string()),
                    ))
                    .child(
                        Button::new("delete-selected-environment")
                            .small()
                            .ghost()
                            .label("Delete")
                            .disabled(self.selected_id.is_none())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.delete_selected_environment(window, cx);
                            })),
                    ),
            );
        variables_panel = variables_panel.child(
            h_flex().w_full().items_center().gap_2().child(
                Input::new(&self.environment_name_input)
                    .w_full()
                    .context_menu({
                        move |menu, _, cx| {
                            build_text_edit_context_menu(
                                menu,
                                environment_name_has_selection,
                                cx.theme().muted_foreground,
                            )
                        }
                    }),
            ),
        );
        if let Some(error) = &self.error {
            variables_panel = variables_panel.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().danger_foreground)
                    .child(error.clone()),
            );
        }
        let mut variables_rows = v_flex().w_full().gap_1().child(
            h_flex()
                .w_full()
                .items_center()
                .gap_2()
                .px_2()
                .py_1()
                .text_xs()
                .font_semibold()
                .text_color(cx.theme().muted_foreground)
                .child(div().w(px(28.0)).child("On"))
                .child(div().w(px(180.0)).child("Key"))
                .child(div().flex_1().child("Value"))
                .child(div().w(px(28.0))),
        );
        variables_rows = variables_rows.child(if self.variables.is_empty() {
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .px_2()
                .py_2()
                .child("No variables yet.")
                .into_any_element()
        } else {
            div().into_any_element()
        });
        variables_rows =
            variables_rows.children(self.variables.iter().enumerate().map(|(index, variable)| {
                let key_input = self.variable_name_inputs[index].clone();
                let value_input = self.variable_value_inputs[index].clone();
                let key_has_selection = !key_input.read(cx).selected_range().is_empty();
                let value_has_selection = !value_input.read(cx).selected_range().is_empty();
                h_flex()
                    .w_full()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(cx.theme().border)
                    .child(
                        div().w(px(28.0)).child(
                            gpui_component::checkbox::Checkbox::new(format!(
                                "env-var-enabled-{index}"
                            ))
                            .small()
                            .checked(variable.enabled)
                            .on_click(cx.listener(
                                move |this, checked: &bool, _, cx| {
                                    if let Some(variable) = this.variables.get_mut(index) {
                                        variable.enabled = *checked;
                                        this.schedule_variables_save(cx);
                                        cx.notify();
                                    }
                                },
                            )),
                        ),
                    )
                    .child(
                        div().w(px(180.0)).child(
                            Input::new(&key_input)
                                .small()
                                .w_full()
                                .appearance(false)
                                .context_menu({
                                    move |menu, _, cx| {
                                        build_text_edit_context_menu(
                                            menu,
                                            key_has_selection,
                                            cx.theme().muted_foreground,
                                        )
                                    }
                                }),
                        ),
                    )
                    .child(
                        div().flex_1().child(
                            Input::new(&value_input)
                                .small()
                                .w_full()
                                .appearance(false)
                                .context_menu({
                                    move |menu, _, cx| {
                                        build_text_edit_context_menu(
                                            menu,
                                            value_has_selection,
                                            cx.theme().muted_foreground,
                                        )
                                    }
                                }),
                        ),
                    )
                    .child(
                        div().w(px(28.0)).child(
                            Button::new(format!("delete-environment-variable-{index}"))
                                .small()
                                .ghost()
                                .cursor_pointer()
                                .icon(
                                    Icon::default()
                                        .path("icons/delete.svg")
                                        .size(px(14.0))
                                        .text_color(cx.theme().muted_foreground),
                                )
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.remove_variable(index, window, cx);
                                })),
                        ),
                    )
            }));
        variables_rows = variables_rows.child(
            h_flex().w_full().justify_end().pt_2().child(
                Button::new("add-environment-variable")
                    .small()
                    .label("Add variable")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.add_variable(window, cx);
                    })),
            ),
        );
        variables_panel = variables_panel.child(div().w_full().child(variables_rows));

        if !self.show_environment_selector {
            if !has_selected_environment {
                return v_flex()
                    .w_full()
                    .h_full()
                    .p_4()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .child(
                        v_flex()
                            .w_full()
                            .max_w(px(360.0))
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_base()
                                    .font_semibold()
                                    .child("No environment selected"),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_center()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(
                                        "Create an environment or select one to manage variables.",
                                    ),
                            ),
                    )
                    .into_any_element();
            }
            return v_flex()
                .w_full()
                .h_full()
                .p_2()
                .child(
                    div()
                        .w_full()
                        .h_full()
                        .overflow_y_scrollbar()
                        .child(variables_panel),
                )
                .into_any_element();
        }

        v_flex()
            .w_full()
            .h(px(520.0))
            .p_3()
            .gap_3()
            .child(
                h_flex()
                    .w_full()
                    .h_full()
                    .gap_3()
                    .child(
                        v_flex()
                            .w(px(260.0))
                            .h_full()
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(cx.theme().border)
                            .bg(cx.theme().background)
                            .overflow_hidden()
                            .p_3()
                            .gap_2()
                            .child(
                                v_flex()
                                    .w_full()
                                    .gap_1()
                                    .child(div().text_xs().font_semibold().child("Environments")),
                            )
                            .child(v_flex().w_full().flex_1().min_h_0().child(
                                div().w_full().h_full().overflow_y_scrollbar().child(
                                    v_flex().w_full().gap_1().children(
                                        self.options.clone().into_iter().map(
                                            |(environment_id, label)| {
                                                let is_current = Some(environment_id)
                                                    == self.active_environment_id;
                                                let mut row_content = h_flex()
                                                    .w_full()
                                                    .items_center()
                                                    .justify_between()
                                                    .gap_2()
                                                    .child(
                                                        div()
                                                            .min_w_0()
                                                            .flex_1()
                                                            .text_sm()
                                                            .line_height(relative(1.0))
                                                            .truncate()
                                                            .child(label),
                                                    );
                                                if is_current {
                                                    row_content = row_content.child(
                                                        Tag::success()
                                                            .small()
                                                            .outline()
                                                            .rounded_full()
                                                            .child("Current"),
                                                    );
                                                }
                                                ListItem::new(format!(
                                                    "environment-manager-select-{environment_id}"
                                                ))
                                                .w_full()
                                                .cursor_pointer()
                                                .rounded(px(8.0))
                                                .px_3()
                                                .py_2()
                                                .selected(Some(environment_id) == self.selected_id)
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        this.selected_id = Some(environment_id);
                                                        this.load_variables(
                                                            environment_id,
                                                            window,
                                                            cx,
                                                        );
                                                        cx.notify();
                                                    },
                                                ))
                                                .child(row_content)
                                            },
                                        ),
                                    ),
                                ),
                            ))
                            .child(
                                Button::new("environment-manager-add-environment")
                                    .small()
                                    .w_full()
                                    .label("Add environment")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.add_environment(window, cx);
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(cx.theme().border)
                            .bg(cx.theme().background)
                            .p_2()
                            .child(if has_selected_environment {
                                div()
                                    .w_full()
                                    .h_full()
                                    .overflow_y_scrollbar()
                                    .child(variables_panel)
                                    .into_any_element()
                            } else {
                                v_flex()
                                    .w_full()
                                    .h_full()
                                    .items_center()
                                    .justify_center()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Select an environment from the left pane.")
                                    .into_any_element()
                            }),
                    ),
            )
            .into_any_element()
    }
}
