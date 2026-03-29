use ironclaw::channels::IncomingMessage;
use ironclaw::tools::callback::{CallbackMetadata, ToolCallbackRegistry};
use std::time::Duration;
use tokio::sync::mpsc;

#[tokio::test]
async fn test_register_and_check_pending() {
    let registry = ToolCallbackRegistry::new(Duration::from_secs(300));

    let meta = CallbackMetadata {
        tool_name: "wallet_transact".into(),
        user_id: "user-1".into(),
        thread_id: Some("thread-1".into()),
        channel: "web".into(),
    };

    registry.register("corr-123".into(), meta).await;
    assert!(registry.is_pending("corr-123").await);
    assert!(!registry.is_pending("nonexistent").await);
}

#[tokio::test]
async fn test_cancel_removes_entry() {
    let registry = ToolCallbackRegistry::new(Duration::from_secs(300));

    let meta = CallbackMetadata {
        tool_name: "wallet_transact".into(),
        user_id: "user-1".into(),
        thread_id: None,
        channel: "cli".into(),
    };

    registry.register("corr-456".into(), meta).await;
    assert!(registry.is_pending("corr-456").await);

    registry.cancel("corr-456").await;
    assert!(!registry.is_pending("corr-456").await);
}

#[tokio::test]
async fn test_cancel_nonexistent_is_noop() {
    let registry = ToolCallbackRegistry::new(Duration::from_secs(300));
    registry.cancel("does-not-exist").await; // should not panic
}

#[tokio::test]
async fn test_resolve_injects_incoming_message() {
    let (tx, mut rx) = mpsc::channel::<IncomingMessage>(16);
    let registry = ToolCallbackRegistry::new(Duration::from_secs(300));

    let meta = CallbackMetadata {
        tool_name: "wallet_transact".into(),
        user_id: "user-1".into(),
        thread_id: Some("thread-1".into()),
        channel: "web".into(),
    };

    registry.register("corr-789".into(), meta).await;
    registry
        .resolve("corr-789", "Transaction confirmed. Tx hash: 0xabc".into(), &tx)
        .await
        .unwrap();

    // Should no longer be pending after resolve
    assert!(!registry.is_pending("corr-789").await);

    // Should have injected a message
    let msg = rx.try_recv().unwrap();
    assert_eq!(msg.channel, "web");
    assert_eq!(msg.user_id, "user-1");
    assert_eq!(msg.thread_id.as_deref(), Some("thread-1"));
    assert!(msg.content.contains("Transaction confirmed"));
    assert!(msg.is_internal());
}

#[tokio::test]
async fn test_resolve_unknown_correlation_id_returns_error() {
    let (tx, _rx) = mpsc::channel::<IncomingMessage>(16);
    let registry = ToolCallbackRegistry::new(Duration::from_secs(300));

    let result = registry
        .resolve("nonexistent", "some result".into(), &tx)
        .await;
    assert!(result.is_err());
}
