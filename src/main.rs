use beam::app_shell::{StartupLoad, start_data_sync_worker, startup_preload};
use beam::paths::BeamPaths;
use beam::storage::workspace_repo::WorkspaceRepository;
use beam::storage::fs_backend::FileSystemStorage;
use beam::ui::run_app;

fn main() {
    let backend = FileSystemStorage::new(BeamPaths::default_user_config());
    let mut memory_storage = WorkspaceRepository::new(backend.clone())
        .expect("failed to load workspace into memory");

    let report = match memory_storage.initialize() {
        Ok(report) => report,
        Err(error) => {
            eprintln!("Failed to initialize Beam foundation: {error}");
            std::process::exit(1);
        }
    };

    // TODO: no need to log this.
    println!(
        "Beam foundation initialized at {}",
        backend.paths.root.display()
    );

    // TODO: no need to log this.
    if report.created_workspace_file {
        println!("Created beam.workspace.toml");
    }
    if report.created_local_state_file {
        println!("Created beam_local/local-state.toml");
    }
    if let Err(error) = memory_storage.bootstrap_sample_workspace_if_needed() {
        eprintln!("Failed to create sample workspace content: {error}");
        std::process::exit(1);
    }

    match startup_preload(&memory_storage, &backend.paths) {
        StartupLoad::Ready { state, messages } => {
            // TODO: delete these log.
            println!(
                "App shell ready: collections/workspace split {:.0}%/{:.0}%, request/response split {:.0}%/{:.0}%",
                state.layout.collections_workspace.ratio() * 100.0,
                (1.0 - state.layout.collections_workspace.ratio()) * 100.0,
                state.layout.request_response.ratio() * 100.0,
                (1.0 - state.layout.request_response.ratio()) * 100.0,
            );
            if let Some(request_id) = state.collections.selected_request_id() {
                println!("Restored last opened request: {request_id}");
            }
            for message in &messages {
                eprintln!("[startup warning] {}", message.text);
            }
            let sync_runtime = start_data_sync_worker(memory_storage);
            run_app(state, messages, sync_runtime);
        }
        StartupLoad::Fatal { message } => {
            eprintln!("[startup fatal] {}", message.text);
            std::process::exit(1);
        }
    }
}
