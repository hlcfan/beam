use beam::app_shell::{StartupLoad, start_data_sync_worker, startup_preload};
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
            // If a V2 single-workspace layout exists, migrate it transparently.
            match registry_repo.migrate_single_workspace_if_needed() {
                Ok(Some(migrated)) => (migrated, false),
                _ => {
                    eprintln!("Failed to initialize workspace registry: {error}");
                    std::process::exit(1);
                }
            }
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

    println!(
        "Beam foundation initialized at {}",
        workspace_paths.root.display()
    );

    let backend = FileSystemStorage::new(workspace_paths.clone());
    let mut memory_storage = WorkspaceRepository::new(backend.clone())
        .expect("failed to load workspace into memory");

    match memory_storage.initialize() {
        Ok(report) => {
            if report.created_workspace_file {
                println!("Created beam.workspace.toml");
            }
            if report.created_local_state_file {
                println!("Created local-state.toml");
            }
        }
        Err(error) => {
            eprintln!("Failed to initialize Beam foundation: {error}");
            std::process::exit(1);
        }
    }

    if let Err(error) = memory_storage.bootstrap_sample_workspace_if_needed() {
        eprintln!("Failed to create sample workspace content: {error}");
        std::process::exit(1);
    }

    match startup_preload(
        &memory_storage,
        &workspace_paths,
        active_entry.as_ref(),
        all_workspaces,
    ) {
        StartupLoad::Ready { state, messages } => {
            println!(
                "App shell ready: collections/workspace split {:.0}%/{:.0}%, request/response split {:.0}%/{:.0}%",
                state.layout.collections_workspace.ratio() * 100.0,
                (1.0 - state.layout.collections_workspace.ratio()) * 100.0,
                state.layout.request_response.ratio() * 100.0,
                (1.0 - state.layout.request_response.ratio()) * 100.0,
            );
            if let Some(request_id) = state.workspace_tree.selected_request_id() {
                println!("Restored last opened request: {request_id}");
            }
            for message in &messages {
                eprintln!("[startup warning] {}", message.text);
            }
            let sync_runtime = start_data_sync_worker(memory_storage, registry, registry_repo);
            run_app(state, messages, sync_runtime, workspace_paths);
        }
        StartupLoad::Fatal { message } => {
            eprintln!("[startup fatal] {}", message.text);
            std::process::exit(1);
        }
    }
}
