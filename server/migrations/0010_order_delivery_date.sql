-- Daily administration filters by delivery day while retaining stable pagination.
CREATE INDEX orders_delivery_date_created_at ON orders(delivery_date,created_at DESC,id DESC);
