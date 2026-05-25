use std::process::ExitCode;

use app::ManagedComponent;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> ExitCode {
    let bootstrap = match server::bootstrap::build_runtime_report_from_args(std::env::args()) {
        Ok(bootstrap) => bootstrap,
        Err(error) => {
            tracing::error!(target: "server", "{error}");
            return ExitCode::from(1);
        }
    };

    for deferred in &bootstrap.deferred_protocols {
        tracing::info!(target: "server",
            "deferred bootstrap protocol {} on {}: {}",
            deferred.protocol, deferred.port_name, deferred.reason
        );
    }

    let runtime = bootstrap.runtime;

    if let Err(error) = runtime.start() {
        tracing::error!(target: "server", "failed to start server runtime: {error}");
        return ExitCode::from(1);
    }

    let report = runtime.transport_report();
    if report.bound_addresses.is_empty() {
        tracing::warn!(target: "server", "server runtime started without bound listeners");
    } else {
        tracing::info!(target: "server", "server runtime listening on {:?}", report.bound_addresses);
    }
    tracing::info!(target: "server",
        "server runtime transport report: {} listener(s), {}",
        report.active_listener_count,
        report.deferred_transport_summary()
    );

    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::error!(target: "server", "failed to wait for shutdown signal: {error}");
        runtime.stop();
        return ExitCode::from(1);
    }

    runtime.stop();
    ExitCode::SUCCESS
}
