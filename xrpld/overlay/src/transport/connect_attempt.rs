//! Outbound peer connection attempts over tokio TCP/TLS plus XRPL HTTP upgrade.

use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use basics::base_uint::Uint256;
use http::StatusCode;
use http::header::CONTENT_LENGTH;
use http::header::{HOST, HeaderValue};
use openssl::ssl::{Ssl, SslConnector};
use serde_json::Value as JsonValue;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::watch;
use tokio::time::timeout;
use tokio_openssl::SslStream;

use crate::handshake::{
    HandshakeContext, build_handshake, make_request, make_shared_value_from_finished_messages,
    parse_http_response, serialize_request, validate_outbound_peer_upgrade,
};
use crate::peer::peer::Peer;
use crate::peer_imp::PeerImp;
use crate::session::PeerSessionStarter;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionStep {
    Init,
    TcpConnect,
    TlsHandshake,
    HttpWrite,
    HttpRead,
    Complete,
    ShutdownStarted,
}

#[derive(Debug, Clone)]
pub struct ConnectAttemptConfig {
    pub server_name: String,
    pub crawl_public: bool,
    pub compr_enabled: bool,
    pub ledger_replay_enabled: bool,
    pub tx_reduce_relay_enabled: bool,
    pub vp_reduce_relay_enabled: bool,
    pub connect_timeout: Duration,
}

impl Default for ConnectAttemptConfig {
    fn default() -> Self {
        Self {
            server_name: "localhost".to_owned(),
            crawl_public: false,
            compr_enabled: false,
            ledger_replay_enabled: true,
            tx_reduce_relay_enabled: false,
            vp_reduce_relay_enabled: false,
            connect_timeout: Duration::from_secs(15),
        }
    }
}

#[derive(Debug)]
pub enum ConnectAttemptError {
    Io(io::Error),
    InvalidServerName,
    InvalidHttp(String),
    Protocol(String),
    /// The overlay already has an active or registered outbound attempt for
    /// this endpoint. This is suppression, not a network failure.
    DuplicateOutboundAttempt,
    Redirect(Vec<SocketAddr>),
    Timeout(ConnectionStep),
}

impl std::fmt::Display for ConnectAttemptError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "{error}"),
            Self::InvalidServerName => write!(formatter, "invalid server name"),
            Self::InvalidHttp(error) => write!(formatter, "{error}"),
            Self::Protocol(error) => write!(formatter, "{error}"),
            Self::DuplicateOutboundAttempt => write!(formatter, "duplicate outbound attempt"),
            Self::Redirect(peers) => {
                if peers.is_empty() {
                    write!(formatter, "http status 503 redirected with no peers")
                } else {
                    write!(
                        formatter,
                        "http status 503 redirected to {} peer(s)",
                        peers.len()
                    )
                }
            }
            Self::Timeout(step) => write!(formatter, "timeout at step {step:?}"),
        }
    }
}

impl std::error::Error for ConnectAttemptError {}

impl From<io::Error> for ConnectAttemptError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Debug)]
pub struct ConnectAttemptResult {
    pub peer: Arc<PeerImp>,
    pub response: http::Response<()>,
    pub negotiated_features: http::HeaderMap,
    pub session: Option<PeerSessionStarter>,
}

type VerifyResponse = Arc<
    dyn Fn(&http::Response<()>, SocketAddr, &Uint256) -> Result<Arc<PeerImp>, ConnectAttemptError>
        + Send
        + Sync,
>;
type SignSession = Arc<dyn Fn(&Uint256) -> Result<String, ConnectAttemptError> + Send + Sync>;

pub struct ConnectAttempt {
    remote_endpoint: SocketAddr,
    config: ConnectAttemptConfig,
    connector: Arc<SslConnector>,
    handshake_context: HandshakeContext,
    sign_session: SignSession,
    verify_response: VerifyResponse,
    stop_requested: watch::Receiver<bool>,
}

impl ConnectAttempt {
    pub fn new(
        remote_endpoint: SocketAddr,
        config: ConnectAttemptConfig,
        connector: Arc<SslConnector>,
        handshake_context: HandshakeContext,
        sign_session: SignSession,
        verify_response: VerifyResponse,
        stop_requested: watch::Receiver<bool>,
    ) -> Self {
        Self {
            remote_endpoint,
            config,
            connector,
            handshake_context,
            sign_session,
            verify_response,
            stop_requested,
        }
    }

    pub async fn run(&self) -> Result<ConnectAttemptResult, ConnectAttemptError> {
        self.run_inner(self.stop_requested.clone()).await
    }

    async fn await_phase<T, F>(
        &self,
        step: ConnectionStep,
        stop_requested: &mut watch::Receiver<bool>,
        operation: F,
    ) -> Result<T, ConnectAttemptError>
    where
        F: std::future::Future<Output = Result<T, ConnectAttemptError>>,
    {
        tokio::select! {
            biased;
            changed = stop_requested.changed() => {
                let _ = changed;
                Err(Self::shutdown_started_error())
            }
            result = timeout(self.config.connect_timeout, operation) => {
                result.map_err(|_| {
                    tracing::warn!(
                        target: "overlay",
                        ip = %self.remote_endpoint,
                        ?step,
                        "Connection attempt phase timed out"
                    );
                    ConnectAttemptError::Timeout(step)
                })?
            }
        }
    }

    async fn run_inner(
        &self,
        mut stop_requested: watch::Receiver<bool>,
    ) -> Result<ConnectAttemptResult, ConnectAttemptError> {
        self.ensure_not_stopping(&stop_requested)?;
        tracing::debug!(target: "overlay", ip = %self.remote_endpoint, "TCP connect starting");
        let tcp_stream = self
            .await_phase(ConnectionStep::TcpConnect, &mut stop_requested, async {
                TcpStream::connect(self.remote_endpoint)
                    .await
                    .map_err(ConnectAttemptError::Io)
            })
            .await?;

        // Disable Nagle's algorithm for low-latency request-response pipelining.
        let _ = tcp_stream.set_nodelay(true);

        let mut ssl = Ssl::new(self.connector.context())
            .map_err(|error| ConnectAttemptError::Protocol(error.to_string()))?;
        ssl.set_hostname(&self.config.server_name)
            .map_err(|_| ConnectAttemptError::InvalidServerName)?;
        let mut tls_stream = SslStream::new(ssl, tcp_stream)
            .map_err(|error| ConnectAttemptError::Protocol(error.to_string()))?;
        self.ensure_not_stopping(&stop_requested)?;
        self.await_phase(ConnectionStep::TlsHandshake, &mut stop_requested, async {
            std::pin::Pin::new(&mut tls_stream)
                .connect()
                .await
                .map_err(|error| ConnectAttemptError::Protocol(error.to_string()))
        })
        .await?;

        let shared_value = make_shared_value(&tls_stream)?;
        tracing::debug!(target: "overlay", ip = %self.remote_endpoint, "TLS handshake complete, sending HTTP upgrade");
        let mut handshake_context = self.handshake_context.clone();
        handshake_context.session_signature = (self.sign_session)(&shared_value)?;
        let mut request = make_request(
            self.config.crawl_public,
            self.config.compr_enabled,
            self.config.ledger_replay_enabled,
            self.config.tx_reduce_relay_enabled,
            self.config.vp_reduce_relay_enabled,
        );
        // HTTP/1.1 peers can reject upgrade requests without Host.
        if let Ok(host) = HeaderValue::from_str(&self.remote_endpoint.to_string()) {
            request.headers_mut().insert(HOST, host);
        }
        build_handshake(request.headers_mut(), &handshake_context);

        let wire_request = serialize_request(&request);
        self.ensure_not_stopping(&stop_requested)?;
        self.await_phase(ConnectionStep::HttpWrite, &mut stop_requested, async {
            tls_stream
                .write_all(&wire_request)
                .await
                .map_err(ConnectAttemptError::Io)
        })
        .await?;
        self.ensure_not_stopping(&stop_requested)?;
        self.await_phase(ConnectionStep::HttpWrite, &mut stop_requested, async {
            tls_stream.flush().await.map_err(ConnectAttemptError::Io)
        })
        .await?;

        self.ensure_not_stopping(&stop_requested)?;
        let http_response = self
            .await_phase(ConnectionStep::HttpRead, &mut stop_requested, async {
                read_http_response(&mut tls_stream)
                    .await
                    .map_err(ConnectAttemptError::Io)
            })
            .await?;
        let response =
            parse_http_response(&http_response.wire).map_err(ConnectAttemptError::InvalidHttp)?;
        if response.status() != StatusCode::SWITCHING_PROTOCOLS {
            if response.status() == StatusCode::SERVICE_UNAVAILABLE
                && let Some(peers) = parse_redirect_peers(&http_response.wire)
            {
                tracing::debug!(
                    target: "overlay",
                    ip = %self.remote_endpoint,
                    redirect_count = peers.len(),
                    "Connection redirected"
                );
                return Err(ConnectAttemptError::Redirect(peers));
            }
            tracing::warn!(
                target: "overlay",
                ip = %self.remote_endpoint,
                status = response.status().as_u16(),
                "Connection attempt rejected"
            );
            return Err(ConnectAttemptError::Protocol(format!(
                "http status {}",
                response.status().as_u16()
            )));
        }
        validate_outbound_peer_upgrade(&response).map_err(ConnectAttemptError::Protocol)?;
        let peer = (self.verify_response)(&response, self.remote_endpoint, &shared_value)?;

        tracing::info!(
            target: "overlay",
            ip = %self.remote_endpoint,
            peer_id = %peer.id(),
            "Outbound connection established"
        );
        let session = PeerSessionStarter::new(Box::new(tls_stream), self.stop_requested.clone())
            .with_initial_buffer(http_response.read_ahead);
        Ok(ConnectAttemptResult {
            negotiated_features: response.headers().clone(),
            peer,
            response,
            session: Some(session),
        })
    }

    fn ensure_not_stopping(
        &self,
        stop_requested: &watch::Receiver<bool>,
    ) -> Result<(), ConnectAttemptError> {
        if *stop_requested.borrow() {
            Err(Self::shutdown_started_error())
        } else {
            Ok(())
        }
    }

    fn shutdown_started_error() -> ConnectAttemptError {
        ConnectAttemptError::Timeout(ConnectionStep::ShutdownStarted)
    }
}

struct HttpResponseRead {
    wire: Vec<u8>,
    read_ahead: Vec<u8>,
}

async fn read_http_response<S>(stream: &mut S) -> Result<HttpResponseRead, io::Error>
where
    S: AsyncReadExt + Unpin,
{
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];
    let mut header_end = None;
    let mut total_len = None;

    loop {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "http response terminated early",
            ));
        }
        buffer.extend_from_slice(&chunk[..read]);

        if header_end.is_none() {
            header_end = find_header_end(&buffer);
            if let Some(end) = header_end {
                total_len = Some(end + parse_content_length(&buffer[..end])?);
            }
        }

        if let Some(expected) = total_len
            && buffer.len() >= expected
        {
            let read_ahead = buffer.split_off(expected);
            return Ok(HttpResponseRead {
                wire: buffer,
                read_ahead,
            });
        }
    }
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
}

fn parse_content_length(head: &[u8]) -> Result<usize, io::Error> {
    let response = parse_http_response(head)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let Some(length) = response.headers().get(CONTENT_LENGTH) else {
        return Ok(0);
    };
    let length = length
        .to_str()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?
        .parse::<usize>()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    Ok(length)
}

fn parse_redirect_peers(response_bytes: &[u8]) -> Option<Vec<SocketAddr>> {
    let header_end = find_header_end(response_bytes)?;
    let body = &response_bytes[header_end..];
    let json = serde_json::from_slice::<JsonValue>(body).ok()?;
    let peers = json.get("peer-ips")?.as_array()?;
    let mut addrs = Vec::new();
    for peer in peers {
        let Some(text) = peer.as_str() else {
            continue;
        };
        let Ok(addr) = text.parse::<SocketAddr>() else {
            continue;
        };
        addrs.push(addr);
    }
    Some(addrs)
}

fn make_shared_value(stream: &SslStream<TcpStream>) -> Result<Uint256, ConnectAttemptError> {
    let ssl = stream.ssl();
    let mut local_finished = [0u8; 64];
    let mut peer_finished = [0u8; 64];
    let local_len = ssl.finished(&mut local_finished);
    let peer_len = ssl.peer_finished(&mut peer_finished);
    make_shared_value_from_finished_messages(
        &local_finished[..local_len],
        &peer_finished[..peer_len],
    )
    .ok_or_else(|| ConnectAttemptError::Protocol("unable to derive shared value".to_owned()))
}

#[cfg(test)]
mod tests {
    use super::read_http_response;
    use tokio::io::{AsyncWriteExt, duplex};

    #[tokio::test]
    async fn http_response_preserves_coalesced_overlay_read_ahead() {
        let response =
            b"HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: XRPL/2.2\r\n\r\n";
        let frame = [0_u8, 0, 0, 1, 0, 3, 0x08];
        let (mut reader, mut writer) = duplex(2048);
        writer.write_all(response).await.expect("write response");
        writer.write_all(&frame).await.expect("write frame");

        let parsed = read_http_response(&mut reader)
            .await
            .expect("read coalesced response");

        assert_eq!(parsed.wire, response);
        assert_eq!(parsed.read_ahead, frame);
    }
}
