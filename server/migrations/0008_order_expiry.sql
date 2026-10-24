ALTER TABLE orders DROP CONSTRAINT orders_status_check;
ALTER TABLE orders ADD CONSTRAINT orders_status_check
    CHECK (status IN ('awaiting_payment', 'new', 'confirmed', 'preparing', 'out_for_delivery', 'delivered', 'cancelled', 'expired'));

ALTER TABLE orders DROP CONSTRAINT orders_online_fulfillment_requires_payment_check;
ALTER TABLE orders ADD CONSTRAINT orders_online_fulfillment_requires_payment_check
    CHECK (payment_method <> 'card' OR payment_status = 'paid' OR status IN ('awaiting_payment', 'cancelled', 'expired'));

ALTER TABLE orders ADD CONSTRAINT orders_expired_online_payment_check
    CHECK (status <> 'expired' OR (payment_method = 'card' AND payment_status = 'unpaid'));
