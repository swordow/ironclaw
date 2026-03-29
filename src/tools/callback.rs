use std::collections::HashMap;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use crate::channels::IncomingMessage;

/// Metadata stored alongside a pending async tool result.
#[derive(Debug, Clone)]
pub struct CallbackMetadata {
    pub tool_name: String,
    pub user_id: String,
    pub thread_id: Option<String>,
    pub channel: String,
}

/// Error type for callback resolution.
#[derive(Debug, thiserror::Error)]
pub enum CallbackError {
    #[error("unknown correlation ID: {0}")]
    UnknownCorrelationId(String),

    #[error("failed to inject message: {0}")]
    InjectionFailed(String),
}

/// Internal entry with timestamp for TTL expiry.
#[derive(Debug)]
struct PendingEntry {
    metadata: CallbackMetadata,
    #[allow(dead_code)]
    registered_at: Instant,
}

/// Registry for async tool results. Tools register a correlation ID when
/// returning a pending result; external backends call resolve() when the
/// result arrives, which injects an IncomingMessage into the channel system.
pub struct ToolCallbackRegistry {
    pending: tokio::sync::RwLock<HashMap<String, PendingEntry>>,
    ttl: Duration,
}

impl ToolCallbackRegistry {
    pub fn new(ttl: Duration) -> Self {
        Self {
            pending: tokio::sync::RwLock::new(HashMap::new()),
            ttl,
        }
    }

    /// Register a pending async tool result.
    pub async fn register(&self, correlation_id: String, metadata: CallbackMetadata) {
        let entry = PendingEntry {
            metadata,
            registered_at: Instant::now(),
        };
        self.pending.write().await.insert(correlation_id, entry);
    }

    /// Check if a correlation ID is pending.
    pub async fn is_pending(&self, correlation_id: &str) -> bool {
        self.pending.read().await.contains_key(correlation_id)
    }

    /// Cancel a pending result (cleanup).
    pub async fn cancel(&self, correlation_id: &str) {
        self.pending.write().await.remove(correlation_id);
    }

    /// Returns the configured TTL.
    pub fn ttl(&self) -> Duration {
        self.ttl
    }

    /// Resolve a pending result, injecting an `IncomingMessage` into the channel system.
    /// Removes the entry from the pending map on success.
    pub async fn resolve(
        &self,
        correlation_id: &str,
        result: String,
        inject_tx: &mpsc::Sender<IncomingMessage>,
    ) -> Result<(), CallbackError> {
        let entry = self
            .pending
            .write()
            .await
            .remove(correlation_id)
            .ok_or_else(|| CallbackError::UnknownCorrelationId(correlation_id.to_string()))?;

        let mut message =
            IncomingMessage::new(entry.metadata.channel, entry.metadata.user_id, result)
                .into_internal();

        if let Some(tid) = entry.metadata.thread_id {
            message = message.with_thread(tid);
        }

        inject_tx.send(message).await.map_err(
            |e: mpsc::error::SendError<IncomingMessage>| {
                CallbackError::InjectionFailed(e.to_string())
            },
        )
    }
}
