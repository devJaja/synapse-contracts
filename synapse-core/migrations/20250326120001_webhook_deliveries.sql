-- Audit trail and retry scheduling for webhook delivery attempts.
CREATE TABLE webhook_deliveries (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    webhook_endpoint_id UUID NOT NULL REFERENCES webhook_endpoints (id) ON DELETE CASCADE,
    event_type TEXT NOT NULL,
    payload JSONB NOT NULL,
    attempt_count INT NOT NULL DEFAULT 0,
    max_attempts INT NOT NULL DEFAULT 5,
    last_attempt_at TIMESTAMPTZ,
    next_retry_at TIMESTAMPTZ,
    status TEXT NOT NULL DEFAULT 'pending',
    last_error TEXT,
    response_status INT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT webhook_deliveries_status_check CHECK (
        status IN ('pending', 'delivered', 'failed')
    )
);

CREATE INDEX idx_webhook_deliveries_endpoint ON webhook_deliveries (webhook_endpoint_id);
CREATE INDEX idx_webhook_deliveries_status_retry
    ON webhook_deliveries (status, next_retry_at)
    WHERE status = 'pending';

COMMENT ON TABLE webhook_deliveries IS 'Per-delivery audit log; supports exponential backoff retries (#81).';
