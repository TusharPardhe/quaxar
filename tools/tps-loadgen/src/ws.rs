//! Pipelined WebSocket JSON client: many requests in flight on one
//! connection, matched to responses by `id`. Requests sent on one connection
//! reach the server in order, which keeps one account's sequence numbers in
//! order.

use futures::{SinkExt, StreamExt};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

pub struct Client {
    tx: mpsc::UnboundedSender<String>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>,
    next_id: AtomicU64,
    /// Non-response messages (subscription streams).
    pub stream: Mutex<mpsc::UnboundedReceiver<Value>>,
}

impl Client {
    pub async fn connect(url: &str) -> Result<Arc<Self>, String> {
        let (socket, _) = tokio_tungstenite::connect_async(url)
            .await
            .map_err(|e| format!("connect {url}: {e}"))?;
        let (mut sink, mut source) = socket.split();
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let (stream_tx, stream_rx) = mpsc::unbounded_channel();
        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>> = Arc::default();

        tokio::spawn(async move {
            while let Some(text) = rx.recv().await {
                if sink.send(Message::Text(text.into())).await.is_err() {
                    break;
                }
            }
        });
        let reader_pending = Arc::clone(&pending);
        tokio::spawn(async move {
            while let Some(Ok(message)) = source.next().await {
                let Message::Text(text) = message else { continue };
                let Ok(value) = serde_json::from_str::<Value>(&text) else { continue };
                match value.get("id").and_then(Value::as_u64) {
                    Some(id) => {
                        if let Some(waiter) = reader_pending.lock().await.remove(&id) {
                            let _ = waiter.send(value);
                        }
                    }
                    None => {
                        let _ = stream_tx.send(value);
                    }
                }
            }
            // Connection closed: fail every waiter.
            reader_pending.lock().await.clear();
        });
        Ok(Arc::new(Self {
            tx,
            pending,
            next_id: AtomicU64::new(1),
            stream: Mutex::new(stream_rx),
        }))
    }

    /// Send `request` (an object without `id`) and wait for its response.
    pub async fn call(&self, mut request: Value) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        request["id"] = Value::from(id);
        let (waiter, response) = oneshot::channel();
        self.pending.lock().await.insert(id, waiter);
        self.tx
            .send(request.to_string())
            .map_err(|_| "connection closed".to_string())?;
        response.await.map_err(|_| "connection closed".to_string())
    }

    /// Queue `request` in send order without waiting; the response future
    /// is returned for the caller to await.
    pub async fn send(&self, mut request: Value) -> Result<oneshot::Receiver<Value>, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        request["id"] = Value::from(id);
        let (waiter, response) = oneshot::channel();
        self.pending.lock().await.insert(id, waiter);
        self.tx
            .send(request.to_string())
            .map_err(|_| "connection closed".to_string())?;
        Ok(response)
    }
}

/// `result` of a response, or its error text.
pub fn result(response: &Value) -> Result<&Value, String> {
    if response.get("status").and_then(Value::as_str) == Some("error") {
        return Err(response
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("error")
            .to_string());
    }
    response.get("result").ok_or_else(|| "no result".to_string())
}
