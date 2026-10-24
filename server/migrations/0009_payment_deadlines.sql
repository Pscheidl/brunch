-- Preserve each order's original deadline across restarts and configuration changes.
ALTER TABLE payment_attempts ADD COLUMN expires_at TIMESTAMPTZ;
UPDATE payment_attempts p SET expires_at=o.created_at+interval '1 hour'
    FROM orders o WHERE o.id=p.order_id;
ALTER TABLE payment_attempts ALTER COLUMN expires_at SET NOT NULL;
CREATE INDEX payment_attempts_expiry ON payment_attempts(expires_at,order_id);
