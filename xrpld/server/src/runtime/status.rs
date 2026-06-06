use std::sync::Arc;

use app::{
    ApplicationRoot, SharedLedgerMasterState, SharedLoadFeeTrack, SharedNetworkOpsState, StopTree,
    server_okay,
};
use axum::body::Body;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::Response;

const STATUS_PAGE_SYSTEM_NAME: &str = "quaxar";

pub trait ServerStatusSource: Send + Sync {
    fn server_okay(&self) -> Result<(), String>;
}

#[derive(Clone)]
pub struct OwnedServerStatusSource {
    elb_support: bool,
    stop_tree: StopTree,
    network_ops_state: Arc<SharedNetworkOpsState>,
    ledger_master_state: Arc<SharedLedgerMasterState>,
    load_fee_track: Arc<SharedLoadFeeTrack>,
}

impl OwnedServerStatusSource {
    pub fn from_application_root(app: &ApplicationRoot) -> Self {
        Self {
            elb_support: app.elb_support_enabled(),
            stop_tree: app.stop_tree().clone(),
            network_ops_state: app.network_ops_state(),
            ledger_master_state: app.ledger_master_state(),
            load_fee_track: app.load_fee_track(),
        }
    }
}

impl ServerStatusSource for OwnedServerStatusSource {
    fn server_okay(&self) -> Result<(), String> {
        server_okay(
            self.elb_support,
            &self.stop_tree,
            self.network_ops_state.as_ref(),
            self.ledger_master_state.is_caught_up(),
            self.load_fee_track.is_loaded_local(),
        )
        .map_err(str::to_owned)
    }
}

pub fn invalid_protocol_response(status: StatusCode) -> Response {
    html_response(status, "Invalid protocol.")
}

pub fn status_page_response(status_source: &dyn ServerStatusSource) -> Response {
    match status_source.server_okay() {
        Ok(()) => html_response(
            StatusCode::OK,
            format!(
                "<!DOCTYPE html><html><head><title>Test page for {0}</title></head><body><h1>Test</h1><p>This page shows {0} http(s) connectivity is working.</p></body></html>",
                STATUS_PAGE_SYSTEM_NAME
            ),
        ),
        Err(reason) => html_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("<HTML><BODY>Server cannot accept clients: {reason}</BODY></HTML>"),
        ),
    }
}

fn html_response(status: StatusCode, body: impl Into<String>) -> Response {
    let mut response = Response::new(Body::from(body.into()));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    response
        .headers_mut()
        .insert(header::CONNECTION, HeaderValue::from_static("close"));
    response
}

#[cfg(test)]
mod tests {
    use super::{
        OwnedServerStatusSource, ServerStatusSource, invalid_protocol_response,
        status_page_response,
    };
    use app::ApplicationRoot;
    use axum::body::to_bytes;
    use axum::http::StatusCode;

    struct FixedStatusSource(Result<(), String>);

    impl ServerStatusSource for FixedStatusSource {
        fn server_okay(&self) -> Result<(), String> {
            self.0.clone()
        }
    }

    #[tokio::test]
    async fn invalid_protocol_response_html_shell() {
        let response = invalid_protocol_response(StatusCode::UNAUTHORIZED);
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body should read");
        assert_eq!(
            std::str::from_utf8(&body).expect("utf8"),
            "Invalid protocol."
        );
    }

    #[tokio::test]
    async fn status_page_response_reports_okay_and_failure() {
        let ok_response = status_page_response(&FixedStatusSource(Ok(())));
        assert_eq!(ok_response.status(), StatusCode::OK);
        let ok_body = to_bytes(ok_response.into_body(), usize::MAX)
            .await
            .expect("body should read");
        let ok_body = std::str::from_utf8(&ok_body).expect("utf8");
        assert!(ok_body.contains("Test page for xrpld"));
        assert!(ok_body.contains("connectivity is working"));

        let error_response =
            status_page_response(&FixedStatusSource(Err("Too much load".to_owned())));
        assert_eq!(error_response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let error_body = to_bytes(error_response.into_body(), usize::MAX)
            .await
            .expect("body should read");
        assert_eq!(
            std::str::from_utf8(&error_body).expect("utf8"),
            "<HTML><BODY>Server cannot accept clients: Too much load</BODY></HTML>"
        );
    }

    #[test]
    fn owned_status_source_reads_live_application_state() {
        let app = ApplicationRoot::with_options(app::ApplicationRootOptions {
            start_valid: true,
            elb_support: true,
            ..app::ApplicationRootOptions::default()
        })
        .expect("app should build");

        let status_source = OwnedServerStatusSource::from_application_root(&app);
        assert!(status_source.server_okay().is_err());
    }
}
