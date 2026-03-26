//! Bridge transaction state machine hook — enqueue outgoing webhooks on terminal states (#81).

use std::sync::Arc;

use serde_json::Value;

use super::webhook_dispatcher::WebhookDispatcher;

/// Call from the transaction pipeline when a bridge tx reaches a terminal state (#3, #10).
#[derive(Clone)]
pub struct TransactionProcessor {
    webhooks: Arc<WebhookDispatcher>,
}

impl TransactionProcessor {
    pub fn new(webhooks: Arc<WebhookDispatcher>) -> Self {
        Self { webhooks }
    }

    /// After on-chain / internal state shows **completed** — notifies `transaction.completed` subscribers.
    pub async fn on_transaction_completed(&self, payload: Value) -> anyhow::Result<()> {
        self.webhooks
            .dispatch_event("transaction.completed", payload)
            .await
    }

    /// After a terminal failure (DLQ, validation, etc.) — notifies `transaction.failed` subscribers.
    pub async fn on_transaction_failed(&self, payload: Value) -> anyhow::Result<()> {
        self.webhooks
            .dispatch_event("transaction.failed", payload)
            .await
    }
}
