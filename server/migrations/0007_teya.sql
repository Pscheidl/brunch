-- Payment work survives process loss. Provider requests use the persisted UUID as Idempotency-Key.
CREATE TABLE payment_attempts (
    order_id UUID PRIMARY KEY REFERENCES orders(id),
    idempotency_key UUID NOT NULL UNIQUE,
    environment TEXT NOT NULL,
    store_id TEXT NOT NULL,
    request_body JSONB NOT NULL,
    session_id TEXT UNIQUE,
    candidate_session_id TEXT,
    checkout_url TEXT,
    state TEXT NOT NULL DEFAULT 'pending' CHECK (state IN ('pending','ready','paid','failed','expired','review')),
    next_check_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    lease_id UUID,
    lease_until TIMESTAMPTZ,
    failures INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    checked_at TIMESTAMPTZ
);
CREATE INDEX payment_attempts_due ON payment_attempts(next_check_at) WHERE state IN ('pending','ready');

CREATE TABLE payment_receipts (
    order_id UUID PRIMARY KEY REFERENCES orders(id),
    transaction_id TEXT NOT NULL UNIQUE,
    session_id TEXT NOT NULL UNIQUE,
    amount_cents BIGINT NOT NULL CHECK (amount_cents > 0),
    currency TEXT NOT NULL CHECK (currency = 'CZK'),
    verified_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Only matching, RSA-verified notifications are recorded. No PAN, card metadata or raw bodies.
CREATE TABLE payment_notifications (
    body_hash TEXT PRIMARY KEY,
    order_id UUID NOT NULL REFERENCES orders(id),
    session_id TEXT NOT NULL,
    transaction_id TEXT NOT NULL,
    received_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
