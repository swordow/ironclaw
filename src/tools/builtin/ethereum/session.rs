use tokio::sync::RwLock;

/// Status of the WalletConnect session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionStatus {
    Disconnected,
    Pairing { uri: String },
    Paired { address: String, chain_id: u64 },
    Expired,
}

/// Manages a WalletConnect v2 session.
pub struct WalletConnectSession {
    status: RwLock<SessionStatus>,
}

impl WalletConnectSession {
    pub fn new_disconnected() -> Self {
        Self {
            status: RwLock::new(SessionStatus::Disconnected),
        }
    }

    pub async fn is_paired(&self) -> bool {
        matches!(*self.status.read().await, SessionStatus::Paired { .. })
    }

    pub async fn active_address(&self) -> Option<String> {
        match &*self.status.read().await {
            SessionStatus::Paired { address, .. } => Some(address.clone()),
            _ => None,
        }
    }

    pub async fn active_chain_id(&self) -> Option<u64> {
        match &*self.status.read().await {
            SessionStatus::Paired { chain_id, .. } => Some(*chain_id),
            _ => None,
        }
    }

    pub async fn status(&self) -> SessionStatus {
        self.status.read().await.clone()
    }

    pub async fn set_status(&self, new_status: SessionStatus) {
        *self.status.write().await = new_status;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_new_session_manager_is_not_paired() {
        let manager = WalletConnectSession::new_disconnected();
        assert!(!manager.is_paired().await);
        assert!(manager.active_address().await.is_none());
    }

    #[tokio::test]
    async fn test_session_status_when_not_paired() {
        let manager = WalletConnectSession::new_disconnected();
        let status = manager.status().await;
        assert!(matches!(status, SessionStatus::Disconnected));
    }
}
