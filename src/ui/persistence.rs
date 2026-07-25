use super::*;

impl BeamView {
    pub(in crate::ui) fn persist_last_opened_request_id(
        &self,
        request_id: Ulid,
    ) -> Result<(), String> {
        let backend = FileSystemStorage::new(self.current_workspace_paths.clone());
        let storage = WorkspaceRepository::new(backend)
            .map_err(|error| format!("Failed to load workspace: {error}"))?;
        let mut local_state = match storage.load_local_state() {
            Ok(state) => state,
            Err(_) => LocalStateFile::default(),
        };

        if local_state.local_state.last_opened_request_id == Some(request_id) {
            return Ok(());
        }

        local_state.local_state.last_opened_request_id = Some(request_id);
        local_state.local_state.updated_at = Utc::now();
        storage
            .save_local_state(&local_state)
            .map_err(|error| format!("Failed to save local state: {error}"))
    }

    pub(in crate::ui) fn persist_tree_expansion_state(&self) -> Result<(), String> {
        let backend = FileSystemStorage::new(self.current_workspace_paths.clone());
        // TODO: can we not initialize WorkspaceRepository everytime
        let storage = WorkspaceRepository::new(backend)
            .map_err(|error| format!("Failed to load workspace: {error}"))?;
        let mut local_state = match storage.load_local_state() {
            Ok(state) => state,
            Err(_) => LocalStateFile::default(),
        };

        let expanded_item_ids: Vec<Ulid> = self
            .shell
            .workspace_tree
            .expanded()
            .iter()
            .copied()
            .collect();
        if local_state.tree_state.expanded_item_ids == expanded_item_ids {
            return Ok(());
        }

        local_state.tree_state.expanded_item_ids = expanded_item_ids;
        local_state.local_state.updated_at = Utc::now();
        storage
            .save_local_state(&local_state)
            .map_err(|error| format!("Failed to save local state: {error}"))
    }

    pub(in crate::ui) fn persist_environment_selection_state(&self) -> Result<(), String> {
        let backend = FileSystemStorage::new(self.current_workspace_paths.clone());
        let storage = WorkspaceRepository::new(backend)
            .map_err(|error| format!("Failed to load workspace: {error}"))?;
        let mut local_state = match storage.load_local_state() {
            Ok(state) => state,
            Err(_) => LocalStateFile::default(),
        };

        let active_global_environment_id = self
            .shell
            .environment_selection
            .active_global_environment_id;
        if local_state.local_state.active_global_environment_id == active_global_environment_id {
            return Ok(());
        }

        local_state.local_state.active_global_environment_id = active_global_environment_id;
        local_state.local_state.updated_at = Utc::now();
        storage
            .save_local_state(&local_state)
            .map_err(|error| format!("Failed to save local state: {error}"))
    }
}
