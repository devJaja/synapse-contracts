-- Outgoing webhook registration (per event type). Secret is used for HMAC-SHA256 signing.
CREATE TABLE webhook_endpoints (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    event_type TEXT NOT NULL,
    url TEXT NOT NULL,
    secret TEXT NOT NULL,
    is_active BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_webhook_endpoints_event_active
    ON webhook_endpoints (event_type)
    WHERE is_active = true;

COMMENT ON TABLE webhook_endpoints IS 'Partner-configured URLs for outgoing webhooks (issue #81).';
