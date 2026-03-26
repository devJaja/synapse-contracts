//! Admin HTTP API to register and manage outgoing webhook endpoints (#81).
//!
//! Depends on admin authentication (#3); uses `Authorization: Bearer <SYNAPSE_ADMIN_TOKEN>` until integrated.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, patch},
};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Clone)]
pub struct AdminState {
    pub pool: PgPool,
    pub admin_token: String,
}

fn authorize(headers: &HeaderMap, token: &str) -> Result<(), StatusCode> {
    let ok = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .is_some_and(|t| t == token);
    if ok {
        Ok(())
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateWebhookBody {
    pub event_type: String,
    pub url: String,
    pub secret: String,
}

#[derive(Debug, Deserialize)]
pub struct PatchWebhookBody {
    pub is_active: Option<bool>,
    pub url: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct WebhookEndpointOut {
    pub id: Uuid,
    pub event_type: String,
    pub url: String,
    /// Last 4 chars of secret for UI; full secret is never returned after create.
    pub secret_hint: String,
    pub is_active: bool,
}

async fn list_webhooks(
    State(state): State<AdminState>,
    headers: HeaderMap,
) -> Result<Json<Vec<WebhookEndpointOut>>, StatusCode> {
    authorize(&headers, &state.admin_token)?;
    let rows = sqlx::query_as::<_, WebhookEndpointRow>(
        r#"
        SELECT id, event_type, url, secret, is_active
        FROM webhook_endpoints
        ORDER BY created_at DESC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .map_err(|e| {
        tracing::error!(%e, "list webhooks");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let out = rows.into_iter().map(WebhookEndpointOut::from).collect();
    Ok(Json(out))
}

#[derive(Debug, sqlx::FromRow)]
struct WebhookEndpointRow {
    id: Uuid,
    event_type: String,
    url: String,
    secret: String,
    is_active: bool,
}

impl From<WebhookEndpointRow> for WebhookEndpointOut {
    fn from(r: WebhookEndpointRow) -> Self {
        let hint = r
            .secret
            .chars()
            .rev()
            .take(4)
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>();
        WebhookEndpointOut {
            id: r.id,
            event_type: r.event_type,
            url: r.url,
            secret_hint: format!("…{hint}"),
            is_active: r.is_active,
        }
    }
}

async fn create_webhook(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Json(body): Json<CreateWebhookBody>,
) -> Result<(StatusCode, Json<WebhookEndpointOut>), StatusCode> {
    authorize(&headers, &state.admin_token)?;
    if body.event_type.is_empty() || body.url.is_empty() || body.secret.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let row = sqlx::query_as::<_, WebhookEndpointRow>(
        r#"
        INSERT INTO webhook_endpoints (event_type, url, secret)
        VALUES ($1, $2, $3)
        RETURNING id, event_type, url, secret, is_active
        "#,
    )
    .bind(&body.event_type)
    .bind(&body.url)
    .bind(&body.secret)
    .fetch_one(&state.pool)
    .await
    .map_err(|e| {
        tracing::error!(%e, "create webhook");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok((StatusCode::CREATED, Json(WebhookEndpointOut::from(row))))
}

async fn patch_webhook(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<PatchWebhookBody>,
) -> Result<StatusCode, StatusCode> {
    authorize(&headers, &state.admin_token)?;
    if body.is_active.is_none() && body.url.is_none() {
        return Err(StatusCode::BAD_REQUEST);
    }

    if let Some(active) = body.is_active {
        sqlx::query(
            r#"
            UPDATE webhook_endpoints
            SET is_active = $2, updated_at = NOW()
            WHERE id = $1
            "#,
        )
        .bind(id)
        .bind(active)
        .execute(&state.pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    if let Some(ref url) = body.url {
        if url.is_empty() {
            return Err(StatusCode::BAD_REQUEST);
        }
        sqlx::query(
            r#"
            UPDATE webhook_endpoints
            SET url = $2, updated_at = NOW()
            WHERE id = $1
            "#,
        )
        .bind(id)
        .bind(url)
        .execute(&state.pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    Ok(StatusCode::NO_CONTENT)
}

async fn delete_webhook(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    authorize(&headers, &state.admin_token)?;
    let r = sqlx::query("DELETE FROM webhook_endpoints WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if r.rows_affected() == 0 {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(StatusCode::NO_CONTENT)
}

pub fn router(state: AdminState) -> Router {
    Router::new()
        .route("/webhooks", get(list_webhooks).post(create_webhook))
        .route(
            "/webhooks/:id",
            patch(patch_webhook).delete(delete_webhook),
        )
        .with_state(state)
}
