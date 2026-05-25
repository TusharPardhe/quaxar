//! Peer session ownership above the negotiated overlay handshake.

use std::sync::Arc;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use crate::Compressed;
use crate::message::{
    Message, MessageHeader, ProtocolMessage, ProtocolMessageError, ProtocolMessageHandler,
    invoke_protocol_message,
};
use crate::overlay_impl::OverlayError;
use crate::peer::{Peer, PeerId};
use crate::peer_imp::PeerImp;

pub trait PeerSessionStream: AsyncRead + AsyncWrite + Unpin + Send {}

impl<T> PeerSessionStream for T where T: AsyncRead + AsyncWrite + Unpin + Send {}

pub type BoxPeerSessionStream = Box<dyn PeerSessionStream>;

pub trait PeerSessionHooks: Send + Sync {
    fn on_message_begin(&self, _peer: &Arc<PeerImp>, _header: &MessageHeader, _compressed: bool) {}
    fn on_message(&self, _peer: &Arc<PeerImp>, _message: &ProtocolMessage) {}
    fn on_message_end(
        &self,
        _peer: &Arc<PeerImp>,
        _header: &MessageHeader,
        _message: &ProtocolMessage,
    ) {
    }
    fn on_message_unknown(&self, _peer: &Arc<PeerImp>, _message_type: u16) {}
    fn on_session_closed(&self, _peer: &Arc<PeerImp>) {}
}

#[allow(dead_code)]
#[derive(Debug, Default)]
pub struct NoopPeerSessionHooks;

impl PeerSessionHooks for NoopPeerSessionHooks {}

pub struct PeerSessionStarter {
    stream: Option<BoxPeerSessionStream>,
    stop_requested: watch::Receiver<bool>,
}

impl std::fmt::Debug for PeerSessionStarter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PeerSessionStarter")
            .finish_non_exhaustive()
    }
}

impl PeerSessionStarter {
    pub fn new(stream: BoxPeerSessionStream, stop_requested: watch::Receiver<bool>) -> Self {
        Self {
            stream: Some(stream),
            stop_requested,
        }
    }

    pub fn start(
        self,
        peer: Arc<PeerImp>,
        hooks: Arc<dyn PeerSessionHooks>,
        on_close: Arc<dyn Fn(PeerId) + Send + Sync>,
    ) -> JoinHandle<Result<(), OverlayError>> {
        self.start_on(&tokio::runtime::Handle::current(), peer, hooks, on_close)
    }

    pub fn start_on(
        mut self,
        handle: &tokio::runtime::Handle,
        peer: Arc<PeerImp>,
        hooks: Arc<dyn PeerSessionHooks>,
        on_close: Arc<dyn Fn(PeerId) + Send + Sync>,
    ) -> JoinHandle<Result<(), OverlayError>> {
        let stream = self.stream.take().expect("peer session stream must exist");
        let stop_requested = self.stop_requested.clone();
        let (session_stop_tx, session_stop_rx) = watch::channel(false);
        let (sender, receiver) = mpsc::unbounded_channel();
        let pending = peer.attach_session(sender.clone(), session_stop_tx);
        tracing::debug!(
            target: "overlay",
            peer_id = %peer.id(),
            pending_messages = pending.len(),
            "Peer session starting"
        );
        handle.spawn(async move {
            let session = PeerSession::new(
                peer,
                stream,
                receiver,
                pending,
                stop_requested,
                session_stop_rx,
                hooks,
                on_close,
            );
            session.run().await
        })
    }
}

struct PeerSession {
    peer: Arc<PeerImp>,
    stream: Option<BoxPeerSessionStream>,
    outbound: mpsc::UnboundedReceiver<Message>,
    pending_outbound: Vec<Message>,
    stop_requested: watch::Receiver<bool>,
    session_stop: watch::Receiver<bool>,
    hooks: Arc<dyn PeerSessionHooks>,
    on_close: Arc<dyn Fn(PeerId) + Send + Sync>,
}

struct PeerSessionCloseGuard {
    peer: Arc<PeerImp>,
    hooks: Arc<dyn PeerSessionHooks>,
    on_close: Arc<dyn Fn(PeerId) + Send + Sync>,
    closed: bool,
}

impl PeerSessionCloseGuard {
    fn new(
        peer: Arc<PeerImp>,
        hooks: Arc<dyn PeerSessionHooks>,
        on_close: Arc<dyn Fn(PeerId) + Send + Sync>,
    ) -> Self {
        Self {
            peer,
            hooks,
            on_close,
            closed: false,
        }
    }

    fn close(&mut self) {
        if self.closed {
            return;
        }
        tracing::debug!(
            target: "overlay",
            peer_id = %self.peer.id(),
            "Session close guard triggered"
        );
        self.peer.detach_session();
        (self.on_close)(self.peer.id());
        self.hooks.on_session_closed(&self.peer);
        self.closed = true;
    }
}

impl Drop for PeerSessionCloseGuard {
    fn drop(&mut self) {
        self.close();
    }
}

impl PeerSession {
    #[allow(clippy::too_many_arguments)]
    fn new(
        peer: Arc<PeerImp>,
        stream: BoxPeerSessionStream,
        outbound: mpsc::UnboundedReceiver<Message>,
        pending_outbound: Vec<Message>,
        stop_requested: watch::Receiver<bool>,
        session_stop: watch::Receiver<bool>,
        hooks: Arc<dyn PeerSessionHooks>,
        on_close: Arc<dyn Fn(PeerId) + Send + Sync>,
    ) -> Self {
        Self {
            peer,
            stream: Some(stream),
            outbound,
            pending_outbound,
            stop_requested,
            session_stop,
            hooks,
            on_close,
        }
    }

    async fn run(mut self) -> Result<(), OverlayError> {
        let mut close_guard = PeerSessionCloseGuard::new(
            Arc::clone(&self.peer),
            Arc::clone(&self.hooks),
            Arc::clone(&self.on_close),
        );
        tracing::info!(
            target: "overlay",
            peer_id = %self.peer.id(),
            ip = %self.peer.remote_address(),
            "Peer connected"
        );
        let result = self.run_inner().await;
        match &result {
            Ok(()) => tracing::info!(
                target: "overlay",
                peer_id = %self.peer.id(),
                ip = %self.peer.remote_address(),
                reason = "clean shutdown",
                "Peer disconnected"
            ),
            Err(error) => {
                tracing::warn!(
                    target: "overlay",
                    peer_id = %self.peer.id(),
                    error = %error,
                    "Bad data from peer"
                );
                tracing::info!(
                    target: "overlay",
                    peer_id = %self.peer.id(),
                    ip = %self.peer.remote_address(),
                    reason = %error,
                    "Peer disconnected"
                );
            }
        }
        close_guard.close();
        result
    }

    async fn run_inner(&mut self) -> Result<(), OverlayError> {
        let stream = self.stream.take().expect("peer session stream must exist");
        let (mut reader, mut writer) = tokio::io::split(stream);
        let mut buffer = Vec::new();
        let mut handler = PeerSessionDispatch::new(Arc::clone(&self.peer), Arc::clone(&self.hooks));

        if *self.stop_requested.borrow() || *self.session_stop.borrow() {
            return Ok(());
        }

        let compression = self.peer.compression_enabled();

        // Send any pending outbound messages before splitting
        for message in self.pending_outbound.drain(..) {
            write_message(&mut writer, &message, compression).await?;
        }

        // Spawn a dedicated writer task — runs independently from the reader.
        // in-flight simultaneously. This split matches that: the reader task
        // dispatches inbound messages while the writer task drains outbound
        // messages, both running concurrently on the tokio runtime.
        let mut outbound_rx = std::mem::replace(
            &mut self.outbound,
            mpsc::unbounded_channel().1, // placeholder — won't be used
        );
        let mut writer_stop = self.stop_requested.clone();
        let mut writer_session_stop = self.session_stop.clone();
        let (writer_dead_tx, mut writer_dead_rx) = watch::channel(false);
        let writer_task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    biased; // prioritize stop signals
                    changed = writer_stop.changed() => {
                        let _ = changed;
                        break;
                    }
                    changed = writer_session_stop.changed() => {
                        let _ = changed;
                        break;
                    }
                    Some(message) = outbound_rx.recv() => {
                        if write_message(&mut writer, &message, compression).await.is_err() {
                            break;
                        }
                        // Drain remaining outbound in one batch
                        while let Ok(message) = outbound_rx.try_recv() {
                            if write_message(&mut writer, &message, compression).await.is_err() {
                                // Signal reader that writer is dead
                                let _ = writer_dead_tx.send(true);
                                return;
                            }
                        }
                    }
                }
            }
            // Signal reader that writer exited
            let _ = writer_dead_tx.send(true);
        });

        // Reader loop — runs independently from the writer task.
        // Reads from the socket, parses messages, dispatches to handlers.
        let read_result = async {
            loop {
                while let Some(consumed) = dispatch_available(&buffer, &mut handler)? {
                    buffer.drain(..consumed);
                }

                if *self.stop_requested.borrow() || *self.session_stop.borrow() {
                    break;
                }

                tokio::select! {
                    result = read_message(&mut reader, &mut buffer) => {
                        match result? {
                            ReadOutcome::EndOfStream => break,
                            ReadOutcome::Progress => {}
                        }
                    }
                    changed = self.stop_requested.changed() => {
                        let _ = changed;
                        break;
                    }
                    changed = self.session_stop.changed() => {
                        let _ = changed;
                        break;
                    }
                    changed = writer_dead_rx.changed() => {
                        // Writer died — tear down the session immediately.
                        // Without this, the peer looks connected but all
                        // outbound requests are silently dropped.
                        let _ = changed;
                        tracing::warn!(
                            target: "overlay",
                            peer_id = %self.peer.id(),
                            "Peer timeout — no response"
                        );
                        break;
                    }
                }
            }
            Ok::<(), OverlayError>(())
        }
        .await;

        // Abort the writer task when the reader exits
        writer_task.abort();
        let _ = writer_task.await;

        read_result
    }
}

enum ReadOutcome {
    Progress,
    EndOfStream,
}

struct PeerSessionDispatch {
    peer: Arc<PeerImp>,
    hooks: Arc<dyn PeerSessionHooks>,
}

impl PeerSessionDispatch {
    fn new(peer: Arc<PeerImp>, hooks: Arc<dyn PeerSessionHooks>) -> Self {
        Self { peer, hooks }
    }
}

impl ProtocolMessageHandler for PeerSessionDispatch {
    fn compression_enabled(&self) -> bool {
        self.peer.compression_enabled()
    }

    fn on_message_begin(&mut self, header: &MessageHeader, compressed: bool) {
        tracing::debug!(
            target: "overlay",
            peer_id = %self.peer.id(),
            msg_type = header.message_type,
            size_bytes = header.total_wire_size,
            "Message received"
        );
        self.hooks.on_message_begin(&self.peer, header, compressed);
    }

    fn on_message(&mut self, message: &ProtocolMessage) {
        self.hooks.on_message(&self.peer, message);
    }

    fn on_message_end(&mut self, header: &MessageHeader, message: &ProtocolMessage) {
        self.hooks.on_message_end(&self.peer, header, message);
    }

    fn on_message_unknown(&mut self, message_type: u16) {
        tracing::warn!(
            target: "overlay",
            peer_id = %self.peer.id(),
            msg_type = message_type,
            "Unknown message type from peer"
        );
        self.hooks.on_message_unknown(&self.peer, message_type);
    }
}

fn dispatch_available(
    buffer: &[u8],
    handler: &mut PeerSessionDispatch,
) -> Result<Option<usize>, OverlayError> {
    let mut hint = 0usize;
    let consumed = invoke_protocol_message(buffer, handler, &mut hint).map_err(|e| {
        tracing::warn!(
            target: "overlay",
            peer_id = %handler.peer.id(),
            error = %e,
            "Bad data from peer"
        );
        session_error(e)
    })?;
    if consumed == 0 {
        Ok(None)
    } else {
        Ok(Some(consumed))
    }
}

async fn read_message<R>(reader: &mut R, buffer: &mut Vec<u8>) -> Result<ReadOutcome, OverlayError>
where
    R: AsyncRead + Unpin,
{
    let mut chunk = [0u8; 65536];
    let read = reader.read(&mut chunk).await.map_err(OverlayError::Io)?;
    if read == 0 {
        return Ok(ReadOutcome::EndOfStream);
    }
    tracing::trace!(target: "overlay", bytes = read, "Raw bytes received");
    buffer.extend_from_slice(&chunk[..read]);
    Ok(ReadOutcome::Progress)
}

async fn write_message<W>(
    writer: &mut W,
    message: &Message,
    compression_enabled: bool,
) -> Result<(), OverlayError>
where
    W: AsyncWrite + Unpin,
{
    let bytes = if compression_enabled {
        message.get_buffer(Compressed::On)
    } else {
        message.get_buffer(Compressed::Off)
    };
    tracing::debug!(
        target: "overlay",
        msg_type = ?message.protocol().message_type,
        size_bytes = bytes.len(),
        "Message sent"
    );
    writer.write_all(bytes).await.map_err(OverlayError::Io)?;
    writer.flush().await.map_err(OverlayError::Io)?;
    Ok(())
}

fn session_error(error: ProtocolMessageError) -> OverlayError {
    OverlayError::InvalidRequest(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::{NoopPeerSessionHooks, PeerSessionHooks, PeerSessionStarter};
    use protocol::{KeyType, SecretKey, derive_public_key};
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt, duplex};
    use tokio::sync::watch;
    use tokio::time::{Duration, timeout};

    use crate::Compressed;
    use crate::message::{
        Message, ProtocolMessage, ProtocolMessageType, ProtocolPayload, TmPing, TmTransaction,
        decode_protocol_message,
    };
    use crate::peer::{Peer, PeerId};
    use crate::peer_imp::PeerImp;

    fn public_key(seed: u8) -> protocol::PublicKey {
        let secret = SecretKey::from_bytes([seed; 32]);
        derive_public_key(KeyType::Secp256k1, &secret).expect("public key")
    }

    fn peer(id: u32) -> Arc<PeerImp> {
        PeerImp::new(
            id,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 5000 + id as u16),
            public_key(id as u8 + 10),
            format!("peer-{id}"),
        )
    }

    #[derive(Default)]
    struct RecordingHooks {
        messages: Mutex<Vec<u16>>,
        closed: AtomicBool,
    }

    impl PeerSessionHooks for RecordingHooks {
        fn on_message(&self, _peer: &Arc<PeerImp>, message: &ProtocolMessage) {
            self.messages
                .lock()
                .expect("messages lock")
                .push(message.message_type as u16);
        }

        fn on_message_unknown(&self, _peer: &Arc<PeerImp>, message_type: u16) {
            self.messages
                .lock()
                .expect("messages lock")
                .push(message_type);
        }

        fn on_session_closed(&self, _peer: &Arc<PeerImp>) {
            self.closed.store(true, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn peer_session_forwards_queued_messages_and_dispatches_incoming_frames() {
        let peer = peer(1);
        let pending = Message::new(
            ProtocolMessage::new(ProtocolPayload::Ping(TmPing {
                r#type: 0,
                seq: Some(7),
                ping_time: None,
                net_time: None,
            })),
            None,
        );
        peer.send(pending.clone());

        let (local, mut remote) = duplex(4096);
        let (stop_requested, stop_rx) = watch::channel(false);
        let hooks = Arc::new(RecordingHooks::default());
        let session = PeerSessionStarter::new(Box::new(local), stop_rx);
        let handle = session.start(
            Arc::clone(&peer),
            hooks.clone(),
            Arc::new(move |_peer_id: PeerId| {}),
        );

        let mut outbound = vec![0u8; pending.get_buffer_size()];
        remote
            .read_exact(&mut outbound)
            .await
            .expect("read outbound");
        let decoded = decode_protocol_message(&outbound, false).expect("decode outbound");
        assert!(matches!(
            decoded.message,
            Some(ProtocolMessage {
                payload: ProtocolPayload::Ping(_),
                ..
            })
        ));
        assert!(peer.queued_messages().is_empty());

        let live = Message::new(
            ProtocolMessage::new(ProtocolPayload::Ping(TmPing {
                r#type: 0,
                seq: Some(8),
                ping_time: None,
                net_time: None,
            })),
            None,
        );
        peer.send(live.clone());
        let mut live_bytes = vec![0u8; live.get_buffer_size()];
        remote.read_exact(&mut live_bytes).await.expect("read live");
        let live_decoded = decode_protocol_message(&live_bytes, false).expect("decode live");
        assert!(matches!(
            live_decoded.message,
            Some(ProtocolMessage {
                payload: ProtocolPayload::Ping(_),
                ..
            })
        ));

        let incoming = Message::new(
            ProtocolMessage::new(ProtocolPayload::Transaction(TmTransaction {
                raw_transaction: vec![1, 2, 3, 4],
                status: 1,
                receive_timestamp: None,
                deferred: None,
            })),
            None,
        );
        remote
            .write_all(incoming.get_buffer(Compressed::Off))
            .await
            .expect("write incoming");
        remote.flush().await.expect("flush incoming");

        timeout(Duration::from_secs(1), async {
            loop {
                if hooks
                    .messages
                    .lock()
                    .expect("messages lock")
                    .contains(&(ProtocolMessageType::MtTransaction as u16))
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("incoming dispatch");

        let _ = stop_requested.send(true);
        handle.await.expect("session join").expect("session");
        assert!(hooks.closed.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn peer_session_stops_when_peer_is_detached() {
        let peer = peer(3);
        let (local, _remote) = duplex(4096);
        let (_stop_requested, stop_rx) = watch::channel(false);
        let hooks = Arc::new(RecordingHooks::default());
        let session = PeerSessionStarter::new(Box::new(local), stop_rx);
        let handle = session.start(
            Arc::clone(&peer),
            hooks.clone(),
            Arc::new(move |_peer_id: PeerId| {}),
        );

        peer.detach_session();
        let result = timeout(Duration::from_secs(1), handle)
            .await
            .expect("session task must stop")
            .expect("session join should succeed");
        assert!(result.is_ok());
        assert!(hooks.closed.load(Ordering::SeqCst));

        let probe = Message::new(
            ProtocolMessage::new(ProtocolPayload::Ping(TmPing {
                r#type: 0,
                seq: Some(9),
                ping_time: None,
                net_time: None,
            })),
            None,
        );
        peer.send(probe);
        assert_eq!(peer.queued_messages().len(), 1);
    }

    #[test]
    fn noop_hooks_compile() {
        let hooks: Arc<dyn PeerSessionHooks> = Arc::new(NoopPeerSessionHooks);
        let peer = peer(2);
        hooks.on_session_closed(&peer);
    }
}
