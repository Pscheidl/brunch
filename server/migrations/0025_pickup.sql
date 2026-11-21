CREATE TABLE pickup_settings (
    id integer PRIMARY KEY CHECK (id=1),
    enabled boolean NOT NULL DEFAULT false,
    name text NOT NULL DEFAULT '',
    street text NOT NULL DEFAULT '',
    city text NOT NULL DEFAULT '',
    postal_code text NOT NULL DEFAULT '',
    instructions text NOT NULL DEFAULT '',
    parking_free boolean NOT NULL DEFAULT true,
    slots text[] NOT NULL DEFAULT '{}',
    version bigint NOT NULL DEFAULT 1
);
INSERT INTO pickup_settings(id) VALUES (1);
ALTER TABLE orders ADD COLUMN fulfillment_method text NOT NULL DEFAULT 'delivery' CHECK (fulfillment_method IN ('delivery','pickup'));
ALTER TABLE orders ADD COLUMN pickup_location jsonb;
ALTER TABLE orders ADD CONSTRAINT orders_pickup_snapshot CHECK (
    (fulfillment_method='delivery' AND pickup_location IS NULL)
    OR (fulfillment_method='pickup' AND pickup_location IS NOT NULL AND jsonb_typeof(pickup_location)='object' AND delivery_fee_cents=0)
);
