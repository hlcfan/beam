use super::*;

#[derive(Clone, Debug)]
pub(super) struct EnvVarHoverInfo {
    var_name: String,
    resolved_value: Option<String>,
    is_dynamic: bool,
    token_bounds: Bounds<Pixels>,
}

impl BeamView {
    pub(in crate::ui) fn active_environment_options(&self) -> Vec<(Ulid, String)> {
        self.shell
            .environments
            .iter()
            .map(|environment| (environment.environment_id, environment.name.clone()))
            .collect()
    }

    pub(in crate::ui) fn selected_environment_id_for_view(&self) -> Option<Ulid> {
        self.shell.effective_environment_id_for_selected_request()
    }

    pub(in crate::ui) fn selected_environment_label(&self) -> String {
        let Some(selected_id) = self.selected_environment_id_for_view() else {
            return "No environment".to_string();
        };
        let Some((_, label)) = self
            .active_environment_options()
            .into_iter()
            .find(|(environment_id, _)| *environment_id == selected_id)
        else {
            return "No environment".to_string();
        };
        label
    }

    pub(in crate::ui) fn set_selected_environment_for_view(&mut self, environment_id: Ulid) {
        self.shell
            .environment_selection
            .active_global_environment_id = Some(environment_id);
    }

    pub(in crate::ui) fn clear_selected_environment_for_view(&mut self) {
        self.shell
            .environment_selection
            .active_global_environment_id = None;
    }

    pub(in crate::ui) fn open_environment_variables_sheet(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let beam_view = cx.entity();
        let workspace_paths = self.current_workspace_paths.clone();
        let selected_option = self
            .selected_environment_id_for_view()
            .and_then(|selected_id| {
                self.shell
                    .environments
                    .iter()
                    .find(|environment| environment.environment_id == selected_id)
                    .map(|environment| {
                        (
                            environment.environment_id,
                            environment.name.clone(),
                            environment.file_name.clone(),
                        )
                    })
            });
        let sheet_view = cx.new(|cx| {
            EnvironmentManagerDialogView::new_for_sheet(
                beam_view.clone(),
                workspace_paths.clone(),
                selected_option.clone(),
                window,
                cx,
            )
        });

        window.open_sheet_at(Placement::Right, cx, move |sheet, _, _| {
            sheet
                .title("Environment Variables")
                .size(px(520.0))
                .child(sheet_view.clone())
        });
    }

    fn environment_manager_options(&self) -> Vec<(Ulid, String)> {
        self.shell
            .environments
            .iter()
            .map(|environment| (environment.environment_id, environment.name.clone()))
            .collect()
    }

    fn environment_manager_file_names(&self) -> HashMap<Ulid, String> {
        self.shell
            .environments
            .iter()
            .map(|environment| (environment.environment_id, environment.file_name.clone()))
            .collect()
    }

    pub(in crate::ui) fn open_environment_manager(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let beam_view = cx.entity();
        let options = self.environment_manager_options();
        let environment_file_names = self.environment_manager_file_names();
        let fallback_id = options.first().map(|(environment_id, _)| *environment_id);
        let selected = self.selected_environment_id_for_view().or(fallback_id);
        let active_environment_id = self.selected_environment_id_for_view();
        let workspace_paths = self.current_workspace_paths.clone();
        let manager_view = cx.new(|cx| {
            EnvironmentManagerDialogView::new(
                beam_view.clone(),
                workspace_paths.clone(),
                options.clone(),
                environment_file_names.clone(),
                selected,
                active_environment_id,
                window,
                cx,
            )
        });
        self.environment_manager_dialog_view = Some(manager_view.clone());
        cx.defer(move |cx| {
            if let Some(root_window) = cx.active_window().and_then(|w| w.downcast::<Root>()) {
                let _ = root_window.update(cx, |_, window, cx| {
                    window.defer(cx, move |window, cx| {
                        window.open_dialog(cx, move |dialog, _, _| {
                            dialog
                                .title("Manage environment")
                                .w(px(920.0))
                                .max_w(px(1200.0))
                                .child(manager_view.clone())
                        });
                    });
                });
            }
        });
        cx.notify();
    }

    pub(in crate::ui) fn refresh_environment_manager_dialog_if_open(
        &mut self,
        latest_upsert: Option<(Ulid, String)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(dialog_view) = self.environment_manager_dialog_view.clone() else {
            return;
        };
        let options = self.environment_manager_options();
        let environment_file_names = self.environment_manager_file_names();
        let active_environment_id = self.selected_environment_id_for_view();
        let workspace_paths = self.current_workspace_paths.clone();
        dialog_view.update(cx, |dialog, cx| {
            dialog.refresh_from_snapshot(
                workspace_paths,
                options,
                environment_file_names,
                active_environment_id,
                latest_upsert,
                window,
                cx,
            );
        });
    }

    pub(in crate::ui) fn invalidate_env_var_resolved_cache(&mut self) {
        self.env_var_resolved_cache = None;
    }

    pub(in crate::ui) fn environment_file_path_from_shell(
        &self,
        environment_id: Ulid,
    ) -> Option<PathBuf> {
        let environment = self
            .shell
            .environments
            .iter()
            .find(|environment| environment.environment_id == environment_id)?;
        environment_file_path_for_workspace(&self.current_workspace_paths, &environment.file_name)
    }

    pub(in crate::ui) fn update_env_var_hover_for_input(
        &mut self,
        input_entity: &Entity<InputState>,
        pos: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let env_id = self.selected_environment_id_for_view();
        let cache_stale = self
            .env_var_resolved_cache
            .as_ref()
            .map(|(cached_id, _)| *cached_id != env_id)
            .unwrap_or(true);
        if cache_stale {
            let env_vars = self.load_environment_for_script(env_id);
            let resolved = build_enabled_environment_lookup(&env_vars);
            self.env_var_resolved_cache = Some((env_id, resolved));
        }

        let found = {
            let input = input_entity.read(cx);
            let text = input.value();
            let line_height = input.line_height().unwrap_or(px(20.));
            let resolved_env = self.env_var_resolved_cache.as_ref().map(|(_, m)| m);

            find_env_var_ranges(text.as_ref())
                .into_iter()
                .find_map(|(byte_range, var_name)| {
                    let bounds = find_token_hover_bounds(input, &byte_range, pos, line_height)?;
                    let resolved_value = resolved_env.and_then(|m| m.get(&var_name).cloned());
                    let is_dynamic = resolved_value.is_none()
                        && crate::template_variables::is_dynamic_variable(&var_name);

                    Some(EnvVarHoverInfo {
                        var_name,
                        resolved_value,
                        is_dynamic,
                        token_bounds: bounds,
                    })
                })
        };

        if self.env_var_hover.as_ref().map(|h| &h.token_bounds)
            != found.as_ref().map(|h| &h.token_bounds)
        {
            self.env_var_hover = found;
            cx.notify();
        }
    }

    pub(in crate::ui) fn clear_env_var_hover(&mut self, cx: &mut Context<Self>) {
        if self.env_var_hover.is_some() {
            self.env_var_hover = None;
            cx.notify();
        }
    }

    pub(in crate::ui) fn render_env_var_hover_overlay(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut elements: Vec<AnyElement> = Vec::new();

        if let Some(hover_info) = &self.env_var_hover {
            let popup_x = hover_info.token_bounds.origin.x;
            let popup_y = hover_info.token_bounds.bottom();
            let var_name = hover_info.var_name.clone();
            let resolved_value = hover_info.resolved_value.clone();
            let is_dynamic = hover_info.is_dynamic;

            let content = h_flex()
                .gap_1p5()
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("{}:", var_name)),
                )
                .child(match &resolved_value {
                    Some(val) => div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .child(val.clone())
                        .into_any_element(),
                    None if is_dynamic => div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .italic()
                        .child("generated when request is sent")
                        .into_any_element(),
                    None => div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .italic()
                        .child("not set")
                        .into_any_element(),
                });

            elements.push(
                deferred(
                    anchored()
                        .snap_to_window_with_margin(px(8.))
                        .anchor(gpui::Anchor::TopLeft)
                        .position(point(popup_x, popup_y))
                        .child(
                            div()
                                .occlude()
                                .popover_style(cx)
                                .px_2()
                                .py_1p5()
                                .child(content),
                        ),
                )
                .with_priority(2)
                .into_any_element(),
            );
        }

        elements
    }
}

pub(super) fn environment_file_path_for_workspace(
    workspace_paths: &BeamPaths,
    file_name: &str,
) -> Option<PathBuf> {
    let trimmed = file_name.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(workspace_paths.environments_dir.join(trimmed))
}

/// Return the visual-line segment of `byte_range` (a `{{var}}` token) that contains `pos`,
/// or `None` if the cursor is not over the token. When soft-wrap splits the token across
/// visual lines, `InputState::range_to_bounds` collapses it into a single rect with negative
/// width, so we walk byte-by-byte and reassemble per-line segments here.
fn find_token_hover_bounds(
    input: &InputState,
    byte_range: &Range<usize>,
    pos: Point<Pixels>,
    line_height: Pixels,
) -> Option<Bounds<Pixels>> {
    let token_bounds = input.range_to_bounds(byte_range)?;
    if token_bounds.size.width > px(0.) && token_bounds.size.height < line_height + px(1.) {
        return token_bounds.contains(&pos).then_some(token_bounds);
    }

    let mut seg_origin: Option<Point<Pixels>> = None;
    let mut seg_right = px(0.);

    let close = |origin: Point<Pixels>, right: Pixels| Bounds {
        origin,
        size: Size {
            width: right - origin.x,
            height: line_height,
        },
    };

    for byte_offset in byte_range.start..byte_range.end {
        let Some(b) = input.range_to_bounds(&(byte_offset..byte_offset + 1)) else {
            continue;
        };

        let byte_wraps = b.size.height > line_height + px(1.) || b.size.width <= px(0.);
        if byte_wraps {
            if let Some(origin) = seg_origin.take() {
                let segment = close(origin, seg_right);
                if segment.contains(&pos) {
                    return Some(segment);
                }
            }
            continue;
        }

        let byte_right = b.origin.x + b.size.width;
        match seg_origin {
            None => {
                seg_origin = Some(b.origin);
                seg_right = byte_right;
            }
            Some(origin) if (b.origin.y - origin.y).abs() < px(1.) => {
                seg_right = byte_right;
            }
            Some(origin) => {
                let segment = close(origin, seg_right);
                if segment.contains(&pos) {
                    return Some(segment);
                }

                seg_origin = Some(b.origin);
                seg_right = byte_right;
            }
        }
    }

    if let Some(origin) = seg_origin {
        let segment = close(origin, seg_right);
        if segment.contains(&pos) {
            return Some(segment);
        }
    }
    None
}

/// Find all `{{var_name}}` tokens in `text`, returning their byte ranges and variable names.
fn find_env_var_ranges(text: &str) -> Vec<(Range<usize>, String)> {
    let mut result = Vec::new();
    let mut index = 0usize;

    while let Some(start_offset) = text[index..].find("{{") {
        let start = index + start_offset;
        let token_start = start + 2;
        let Some(end_offset) = text[token_start..].find("}}") else {
            break;
        };
        let end = token_start + end_offset;
        let var_name = text[token_start..end].trim().to_string();
        if !var_name.is_empty() {
            result.push((start..end + 2, var_name));
        }
        index = end + 2;
    }

    result
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::environment_file_path_for_workspace;
    use crate::paths::BeamPaths;

    #[test]
    fn environment_file_path_uses_selected_workspace_directory() {
        let workspace_paths =
            BeamPaths::from_root(PathBuf::from("/tmp/beam-tests/other-workspace"));

        assert_eq!(
            environment_file_path_for_workspace(&workspace_paths, "prod.env.toml"),
            Some(
                PathBuf::from("/tmp/beam-tests/other-workspace")
                    .join("environments")
                    .join("prod.env.toml")
            )
        );
        assert_eq!(
            environment_file_path_for_workspace(&workspace_paths, "   "),
            None
        );
    }
}
