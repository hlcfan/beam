use beam::app_shell::{StartupLoad, start_data_sync_worker, startup_preload};
use beam::logger::init_logging;
use beam::paths::DataRootPaths;
use beam::storage::fs_backend::FileSystemStorage;
use beam::storage::registry_repo::RegistryRepository;
use beam::storage::workspace_repo::WorkspaceRepository;
use beam::ui::run_app;

fn main() {
    let data_root = DataRootPaths::default_user_config();
    let registry_repo = RegistryRepository::new(data_root.clone());

    let (registry, _created_new) = match registry_repo.initialize() {
        Ok(result) => result,
        Err(error) => {
            eprintln!("Failed to initialize workspace registry: {error}");
            std::process::exit(1);
        }
    };

    let active_entry = registry_repo.active_workspace_entry(&registry).cloned();
    let all_workspaces = registry.registry.workspaces.clone();

    let workspace_paths = match &active_entry {
        Some(entry) => registry_repo.workspace_paths(entry),
        None => {
            eprintln!("No workspace found in registry. Please re-initialize beam.");
            std::process::exit(1);
        }
    };

    if let Err(error) = init_logging(workspace_paths.log_file.clone()) {
        eprintln!("{error}");
    }

    log::info!(
        "beam_foundation_initialized workspace_root={}",
        workspace_paths.root.display()
    );

    let backend = FileSystemStorage::new(workspace_paths.clone());
    let mut memory_storage =
        WorkspaceRepository::new(backend.clone()).expect("failed to load workspace into memory");

    match memory_storage.initialize() {
        Ok(report) => {
            if report.created_workspace_file {
                log::info!("created beam.workspace.toml");
            }
            if report.created_local_state_file {
                log::info!("created local-state.toml");
            }
            if report.created_app_settings_file {
                log::info!("created app-settings.toml");
            }
            if report.created_default_environment {
                log::info!("created default environment");
            }
        }
        Err(error) => {
            log::error!("Failed to initialize Beam foundation: {error}");
            std::process::exit(1);
        }
    }

    if let Err(error) = memory_storage.bootstrap_sample_workspace_if_needed() {
        log::error!("Failed to create sample workspace content: {error}");
        std::process::exit(1);
    }

    match startup_preload(
        &memory_storage,
        &workspace_paths,
        active_entry.as_ref(),
        all_workspaces,
    ) {
        StartupLoad::Ready { state, messages } => {
            for message in &messages {
                log::warn!("[startup warning] {}", message.text);
            }
            let sync_runtime = start_data_sync_worker(memory_storage, registry, registry_repo);
            run_app(state, messages, sync_runtime, workspace_paths);
        }
        StartupLoad::Fatal { message } => {
            log::error!("[startup fatal] {}", message.text);
            std::process::exit(1);
        }
    }
}
