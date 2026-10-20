use super::*;
use crate::models::UpdateChannel;
use crate::updater::{self, AvailableUpdate};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum UpdatePhase {
    Idle,
    ChangingChannel,
    Checking,
    Installing,
    Installed,
    Restarting,
}

pub(super) struct AppUpdater {
    pub(super) channel: UpdateChannel,
    pub(super) phase: UpdatePhase,
    pub(super) release: Option<AvailableUpdate>,
    pub(super) error: Option<String>,
    paths: BeamPaths,
    revision: u64,
    task: Option<Task<()>>,
}

impl AppUpdater {
    pub(super) fn new(channel: UpdateChannel, paths: BeamPaths) -> Self {
        Self {
            channel,
            paths,
            phase: UpdatePhase::Idle,
            release: None,
            error: None,
            revision: 0,
            task: None,
        }
    }

    pub(super) fn channel_locked(&self) -> bool {
        matches!(
            self.phase,
            UpdatePhase::ChangingChannel
                | UpdatePhase::Installing
                | UpdatePhase::Installed
                | UpdatePhase::Restarting
        )
    }

    pub(super) fn check(&mut self, cx: &mut Context<Self>) {
        if !self.channel_locked() && self.phase != UpdatePhase::Checking {
            self.start_check(self.channel, false, cx);
        }
    }

    pub(super) fn select_channel(&mut self, channel: UpdateChannel, cx: &mut Context<Self>) {
        if channel != self.channel && !self.channel_locked() {
            self.start_check(channel, true, cx);
        }
    }

    pub(super) fn install(&mut self, cx: &mut Context<Self>) {
        if self.channel_locked() || self.phase == UpdatePhase::Checking {
            return;
        }
        let Some(release) = self.release.clone().filter(AvailableUpdate::is_available) else {
            return;
        };
        self.phase = UpdatePhase::Installing;
        self.error = None;
        self.task = Some(cx.spawn(async move |view, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { updater::install(release) })
                .await;
            let _ = view.update(cx, |this, cx| {
                match result {
                    Ok(()) => this.phase = UpdatePhase::Installed,
                    Err(error) => {
                        this.phase = UpdatePhase::Idle;
                        this.error = Some(error);
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn start_check(&mut self, channel: UpdateChannel, persist: bool, cx: &mut Context<Self>) {
        let previous_channel = self.channel;
        self.channel = channel;
        self.revision += 1;
        let revision = self.revision;
        self.phase = if persist {
            UpdatePhase::ChangingChannel
        } else {
            UpdatePhase::Checking
        };
        self.release = None;
        self.error = None;
        let paths = self.paths.clone();
        self.task = Some(cx.spawn(async move |view, cx| {
            if persist {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        WorkspaceRepository::new(FileSystemStorage::new(paths))
                            .and_then(|storage| storage.persist_update_channel(channel))
                            .map_err(|error| format!("Could not save the update channel: {error}"))
                    })
                    .await;
                let proceed = view
                    .update(cx, |this, cx| {
                        if this.revision != revision {
                            return false;
                        }
                        match result {
                            Ok(()) => this.phase = UpdatePhase::Checking,
                            Err(error) => {
                                this.channel = previous_channel;
                                this.phase = UpdatePhase::Idle;
                                this.error = Some(error);
                                cx.notify();
                                return false;
                            }
                        }
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !proceed {
                    return;
                }
            }
            let result = cx
                .background_executor()
                .spawn(async move { updater::check(channel) })
                .await;
            let _ = view.update(cx, |this, cx| {
                if this.revision != revision {
                    return;
                }
                this.phase = UpdatePhase::Idle;
                match result {
                    Ok(release) => this.release = Some(release),
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
}

impl BeamView {
    pub(super) fn restart_after_update(&mut self, cx: &mut Context<Self>) {
        if self.updater.read(cx).phase != UpdatePhase::Installed {
            return;
        }
        let result = self.prepare_update_restart();
        self.updater.update(cx, |updater, cx| {
            match result {
                Ok(command_id) => {
                    self.update_restart_command = Some(command_id);
                    updater.phase = UpdatePhase::Restarting;
                    updater.error = None;
                }
                Err(error) => updater.error = Some(error),
            }
            cx.notify();
        });
    }

    fn prepare_update_restart(&mut self) -> Result<String, String> {
        if self
            .request_execution_states
            .values()
            .any(|state| state.cancel_tx.is_some())
        {
            return Err("Wait for active requests to finish before restarting.".into());
        }
        if let Some((request_id, pane_data)) = self.sync_selected_request_pane_data() {
            self.refresh_active_request_cache();
            let request_file = self.build_request_snapshot_for_save(request_id, pane_data)?;
            self.publish_app_command(AppCommand::SaveRequest {
                request_file,
                command_id: next_command_id(),
            })?;
        }
        let command_id = next_command_id();
        self.publish_app_command(AppCommand::Flush {
            command_id: command_id.clone(),
        })?;
        self.pending_request_save_due_at = None;
        Ok(command_id)
    }

    pub(super) fn handle_update_restart_event(&mut self, event: &AppEvent, cx: &mut Context<Self>) {
        let Some(pending) = self.update_restart_command.as_ref() else {
            return;
        };
        match event {
            AppEvent::SyncCompleted { command_id, .. } if command_id == pending => {
                self.update_restart_command = None;
                cx.restart();
            }
            AppEvent::SyncFailed { error, .. } => {
                self.update_restart_command = None;
                self.updater.update(cx, |updater, cx| {
                    updater.phase = UpdatePhase::Installed;
                    updater.error =
                        Some(format!("Could not save changes before restarting: {error}"));
                    cx.notify();
                });
            }
            _ => {}
        }
    }
}
