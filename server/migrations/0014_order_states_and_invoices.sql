-- Unsent orders in preparation remain confirmed and retain their complete history.
WITH promoted AS (
    UPDATE orders SET status='confirmed', version=version+1, updated_at=now()
    WHERE status='preparing' RETURNING id, payment_status
)
INSERT INTO order_history(order_id, status, payment_status)
SELECT id, 'confirmed', payment_status FROM promoted;

ALTER TABLE orders DROP CONSTRAINT orders_status_check;
ALTER TABLE orders ADD CONSTRAINT orders_status_check CHECK (
    status IN ('awaiting_payment','new','confirmed','out_for_delivery','delivered','cancelled','expired')
);

CREATE TABLE invoice_supplier (
    id integer PRIMARY KEY CHECK (id=1),
    name text NOT NULL DEFAULT '',
    address text NOT NULL DEFAULT '',
    company_id text NOT NULL DEFAULT '',
    tax_id text NOT NULL DEFAULT '',
    registration text NOT NULL DEFAULT '',
    vat_payer boolean NOT NULL DEFAULT false,
    version bigint NOT NULL DEFAULT 1
);
INSERT INTO invoice_supplier(id) VALUES (1);

CREATE SEQUENCE invoice_number_seq;
CREATE TABLE invoices (
    order_id uuid PRIMARY KEY REFERENCES orders(id),
    number text NOT NULL UNIQUE,
    issued_by uuid NOT NULL REFERENCES users(id),
    issued_at timestamptz NOT NULL DEFAULT now(),
    document jsonb NOT NULL
);
