//! Outgoing webhook delivery with HMAC-SHA256 signing and exponential backoff (max 5 attempts).

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use hmac::{Hmac, Mac};
use serde_json::Value;
use sha2::Sha256;
use sqlx::types::Json;
use sqlx::{FromRow, PgPool, Row};
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, FromRow)]
struct DeliveryJob {
    url: String,
    secret: String,
    payload: Json<Value>,
    max_attempts: i32,
}

/// Hex-encoded HMAC-SHA256 over the raw JSON body bytes (consumer verifies with shared secret).
pub fn sign_payload(secret: &str, body: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts key of any size");
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

#[derive(Clone)]
pub struct WebhookDispatcher {
    pool: PgPool,
    http: reqwest::Client,
}

impl WebhookDispatcher {
    pub fn new(pool: PgPool) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("build reqwest client");
        Self { pool, http }
    }

    pub fn into_arc(self) -> Arc<Self> {
        Arc::new(self)
    }

    /// Insert one `webhook_deliveries` row per active endpoint for `event_type` and return delivery ids.
    pub async fn enqueue_deliveries(&self, event_type: &str, payload: Value) -> anyhow::Result<Vec<Uuid>> {
        let rows = sqlx::query(
            r#"
            SELECT id FROM webhook_endpoints
            WHERE event_type = $1 AND is_active = true
            "#,
        )
        .bind(event_type)
        .fetch_all(&self.pool)
        .await
        .context("list webhook endpoints")?;

        let payload = Json(payload);
        let mut out = Vec::new();
        for row in rows {
            let endpoint_id: Uuid = row.try_get("id")?;
            let id: Uuid = sqlx::query_scalar(
                r#"
                INSERT INTO webhook_deliveries (webhook_endpoint_id, event_type, payload)
                VALUES ($1, $2, $3)
                RETURNING id
                "#,
            )
            .bind(endpoint_id)
            .bind(event_type)
            .bind(&payload)
            .fetch_one(&self.pool)
            .await
            .context("insert webhook_deliveries")?;
            out.push(id);
        }
        Ok(out)
    }

    pub fn spawn_deliver(self: &Arc<Self>, delivery_id: Uuid) {
        let this = Arc::clone(self);
        tokio::spawn(async move {
            if let Err(e) = this.run_delivery_with_retries(delivery_id).await {
                tracing::error!(%delivery_id, err = %e, "webhook delivery failed");
            }
        });
    }

    /// Enqueue rows and start background delivery tasks.
    pub async fn dispatch_event(self: &Arc<Self>, event_type: &str, payload: Value) -> anyhow::Result<()> {
        let ids = self.enqueue_deliveries(event_type, payload).await?;
        for id in ids {
            self.spawn_deliver(id);
        }
        Ok(())
    }

    async fn run_delivery_with_retries(&self, delivery_id: Uuid) -> anyhow::Result<()> {
        let job = sqlx::query_as::<_, DeliveryJob>(
            r#"
            SELECT e.url, e.secret, d.payload, d.max_attempts
            FROM webhook_deliveries d
            JOIN webhook_endpoints e ON e.id = d.webhook_endpoint_id
            WHERE d.id = $1 AND d.status = 'pending'
            "#,
        )
        .bind(delivery_id)
        .fetch_optional(&self.pool)
        .await
        .context("load delivery job")?
        .ok_or_else(|| anyhow::anyhow!("delivery not pending or missing"))?;

        let DeliveryJob {
            url,
            secret,
            payload,
            max_attempts,
        } = job;

        let max_attempts = max_attempts.clamp(1, 5);
        let body = serde_json::to_vec(&payload.0).context("serialize payload")?;
        let signature = sign_payload(&secret, &body);

        for attempt in 1..=max_attempts {
            if attempt > 1 {
                let wait_secs = 2u64.pow((attempt - 2) as u32);
                tracing::debug!(%delivery_id, attempt, wait_secs, "webhook retry backoff");
                tokio::time::sleep(Duration::from_secs(wait_secs)).await;
            }

            sqlx::query(
                r#"
                UPDATE webhook_deliveries
                SET attempt_count = $2,
                    last_attempt_at = NOW(),
                    next_retry_at = NULL
                WHERE id = $1
                "#,
            )
            .bind(delivery_id)
            .bind(attempt)
            .execute(&self.pool)
            .await
            .context("update attempt_count")?;

            let response = self
                .http
                .post(&url)
                .header("Content-Type", "application/json")
                .header("X-Synapse-Signature", format!("v1={signature}"))
                .body(body.clone())
                .send()
                .await;

            match response {
                Ok(res) => {
                    let code = res.status().as_u16() as i32;
                    if res.status().is_success() {
                        sqlx::query(
                            r#"
                            UPDATE webhook_deliveries
                            SET status = 'delivered',
                                response_status = $2,
                                last_error = NULL
                            WHERE id = $1
                            "#,
                        )
                        .bind(delivery_id)
                        .bind(code)
                        .execute(&self.pool)
                        .await
                        .context("mark delivered")?;
                        return Ok(());
                    }

                    let err = format!("HTTP {}", code);
                    if attempt >= max_attempts {
                        sqlx::query(
                            r#"
                            UPDATE webhook_deliveries
                            SET status = 'failed',
                                response_status = $2,
                                last_error = $3
                            WHERE id = $1
                            "#,
                        )
                        .bind(delivery_id)
                        .bind(code)
                        .bind(&err)
                        .execute(&self.pool)
                        .await
                        .context("mark failed (http)")?;
                        return Ok(());
                    }

                    let next_backoff = 2_i32.pow((attempt - 1) as u32).min(512);
                    sqlx::query(
                        r#"
                        UPDATE webhook_deliveries
                        SET response_status = $2,
                            last_error = $3,
                            next_retry_at = NOW() + ($4 * INTERVAL '1 second')
                        WHERE id = $1
                        "#,
                    )
                    .bind(delivery_id)
                    .bind(code)
                    .bind(&err)
                    .bind(next_backoff)
                    .execute(&self.pool)
                    .await
                    .context("record retry metadata")?;
                }
                Err(e) => {
                    let err = e.to_string();
                    if attempt >= max_attempts {
                        sqlx::query(
                            r#"
                            UPDATE webhook_deliveries
                            SET status = 'failed',
                                last_error = $2
                            WHERE id = $1
                            "#,
                        )
                        .bind(delivery_id)
                        .bind(&err)
                        .execute(&self.pool)
                        .await
                        .context("mark failed (transport)")?;
                        return Ok(());
                    }
                    sqlx::query(
                        r#"
                        UPDATE webhook_deliveries
                        SET last_error = $2,
                            next_retry_at = NOW() + ($3 * INTERVAL '1 second')
                        WHERE id = $1
                        "#,
                    )
                    .bind(delivery_id)
                    .bind(&err)
                    .bind(2_i32.pow((attempt - 1) as u32).min(512))
                    .execute(&self.pool)
                    .await
                    .context("record transport error")?;
                }
            }
        }

        Ok(())
    }
}
