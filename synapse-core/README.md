# synapse-core

Bridge backend service: **outgoing webhooks** for terminal transaction events (issue **#81**).

## Features

- **Postgres** tables `webhook_endpoints` and `webhook_deliveries` (see `migrations/`).
- **Admin API** (`/admin/webhooks`) — register URLs per `event_type` (e.g. `transaction.completed`, `transaction.failed`). Bearer auth: `SYNAPSE_ADMIN_TOKEN` (placeholder until **#3** / **#10**).
- **HMAC-SHA256** signing header: `X-Synapse-Signature: v1=<hex>` over the raw JSON body.
- **Retries**: up to **5** attempts with exponential backoff (1s, 2s, 4s, 8s between attempts).
- **`WebhookDispatcher`** + **`TransactionProcessor`** — call `on_transaction_completed` / `on_transaction_failed` from your transaction pipeline when states become terminal.

## Run

```bash
cp .env.example .env
# create DB `synapse` and ensure Postgres is running
cargo run
```

- `GET /health`
- `GET|POST /admin/webhooks` — list / create (requires `Authorization: Bearer <token>`)
- `PATCH|DELETE /admin/webhooks/:id` — update `is_active` / `url`, or remove endpoint

## Integrate

```rust
use std::sync::Arc;
use synapse_core::{TransactionProcessor, WebhookDispatcher};

let dispatcher = WebhookDispatcher::new(pool.clone()).into_arc();
let processor = TransactionProcessor::new(Arc::clone(&dispatcher));

// On terminal success / failure from your worker:
processor.on_transaction_completed(serde_json::json!({ "tx_id": "..." })).await?;
processor.on_transaction_failed(serde_json::json!({ "tx_id": "...", "reason": "..." })).await?;
```
