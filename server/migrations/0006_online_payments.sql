ALTER TABLE orders DROP CONSTRAINT orders_payment_method_check;
ALTER TABLE orders ADD CONSTRAINT orders_payment_method_check
    CHECK (payment_method IN ('card', 'card_on_delivery', 'cash_on_delivery'));

ALTER TABLE orders DROP CONSTRAINT orders_status_check;
ALTER TABLE orders ADD CONSTRAINT orders_status_check
    CHECK (status IN ('awaiting_payment', 'new', 'confirmed', 'preparing', 'out_for_delivery', 'delivered', 'cancelled'));

ALTER TABLE orders ADD CONSTRAINT orders_awaiting_online_payment_check
    CHECK (status <> 'awaiting_payment' OR (payment_method = 'card' AND payment_status = 'unpaid'));
ALTER TABLE orders ADD CONSTRAINT orders_online_fulfillment_requires_payment_check
    CHECK (payment_method <> 'card' OR payment_status = 'paid' OR status IN ('awaiting_payment', 'cancelled'));
ALTER TABLE orders ADD CONSTRAINT orders_online_positive_total_check
    CHECK (payment_method <> 'card' OR total_cents > 0);
