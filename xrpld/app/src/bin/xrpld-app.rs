use std::process::ExitCode;

fn main() -> ExitCode {
    let bootstrap = match app::build_bootstrap_runtime_from_args(std::env::args()) {
        Ok(bootstrap) => bootstrap,
        Err(error) => {
            tracing::error!(target: "app", "{error}");
            return ExitCode::from(1);
        }
    };

    tracing::info!(target: "app",
        "bootstrapped xrpld app runtime from {} (overlay_runtime={}, overlay_network_id={:?}, cluster_nodes={}, node_family={}, server_ports={}, shamap_store={}, io_threads={}, job_queue_threads={}, ledger_history={}, path_search_old={}, path_search={}, path_search_fast={}, path_search_max={}, fd_required={})",
        bootstrap.report.config_path.display(),
        bootstrap.report.has_overlay_runtime,
        bootstrap.report.overlay_network_id,
        bootstrap.report.cluster_node_count,
        bootstrap.report.has_node_family,
        bootstrap.report.has_server_ports_setup,
        bootstrap.report.has_shamap_store_service,
        bootstrap.report.io_threads,
        bootstrap.report.job_queue_threads,
        bootstrap.report.ledger_history,
        bootstrap.report.path_search_old,
        bootstrap.report.path_search,
        bootstrap.report.path_search_fast,
        bootstrap.report.path_search_max,
        bootstrap.report.fd_required,
    );

    match app::run_bootstrap_runtime(bootstrap) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(target: "app", "{error}");
            ExitCode::from(1)
        }
    }
}
