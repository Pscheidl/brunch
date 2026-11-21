CREATE TABLE customer_addresses (
    id uuid PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES users(id),
    label_key text NOT NULL,
    label text NOT NULL CHECK (length(label) BETWEEN 1 AND 80),
    name text NOT NULL,
    phone text NOT NULL,
    organization text NOT NULL DEFAULT '',
    street text NOT NULL,
    city text NOT NULL,
    postal_code text NOT NULL,
    notes text NOT NULL DEFAULT '',
    version bigint NOT NULL DEFAULT 1,
    updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX customer_addresses_label ON customer_addresses(user_id, label_key);

CREATE TABLE customer_billing (
    user_id uuid PRIMARY KEY REFERENCES users(id),
    name text NOT NULL,
    address text NOT NULL,
    company_id text NOT NULL DEFAULT '',
    tax_id text NOT NULL DEFAULT '',
    version bigint NOT NULL DEFAULT 1
);

CREATE TABLE order_billing (
    order_id uuid PRIMARY KEY REFERENCES orders(id),
    name text NOT NULL,
    address text NOT NULL,
    company_id text NOT NULL DEFAULT '',
    tax_id text NOT NULL DEFAULT '',
    version bigint NOT NULL DEFAULT 1
);

CREATE TABLE customer_order_requests (
    id uuid PRIMARY KEY,
    idempotency_key uuid NOT NULL,
    user_id uuid NOT NULL REFERENCES users(id),
    order_id uuid NOT NULL REFERENCES orders(id),
    kind text NOT NULL CHECK (kind IN ('change', 'cancel')),
    message text NOT NULL CHECK (length(message) BETWEEN 1 AND 2000),
    status text NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'resolved', 'rejected')),
    response text NOT NULL DEFAULT '',
    version bigint NOT NULL DEFAULT 1,
    created_at timestamptz NOT NULL DEFAULT now(),
    resolved_at timestamptz,
    resolved_by uuid REFERENCES users(id),
    UNIQUE (user_id, idempotency_key)
);
CREATE UNIQUE INDEX customer_order_request_pending ON customer_order_requests(order_id) WHERE status='pending';
CREATE INDEX customer_order_request_created ON customer_order_requests(created_at DESC);
