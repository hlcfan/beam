use beam::app_shell::{StartupLoad, startup_preload};
use beam::paths::BeamPaths;
use beam::storage::WorkspaceStorage;
use beam::storage::toml_backend::TomlWorkspaceStorage;
use beam::ui::run_app;

fn main() {
    let storage = TomlWorkspaceStorage::new(BeamPaths::default_user_config());

    let report = match storage.initialize() {
        Ok(report) => report,
        Err(error) => {
            eprintln!("Failed to initialize Beam foundation: {error}");
            std::process::exit(1);
        }
    };

    println!(
        "Beam foundation initialized at {}",
        storage.paths.root.display()
    );
    if report.created_workspace_file {
        println!("Created beam.workspace.toml");
    }
    if report.created_local_state_file {
        println!("Created .beam/local-state.toml");
    }

    match startup_preload(&storage, &storage.paths) {
        StartupLoad::Ready { state, messages } => {
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
            run_app(state, messages);
        }
        StartupLoad::Fatal { message } => {
            eprintln!("[startup fatal] {}", message.text);
            std::process::exit(1);
        }
    }
}
