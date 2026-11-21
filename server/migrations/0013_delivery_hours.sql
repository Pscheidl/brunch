ALTER TABLE shop_settings
    ADD COLUMN delivery_slots text[] NOT NULL
    DEFAULT ARRAY['07:00–09:00','09:00–11:00','11:00–13:00']::text[]
    CHECK (cardinality(delivery_slots) > 0);

-- NULL inherits the current shop-wide schedule, including for existing areas.
ALTER TABLE delivery_areas
    ADD COLUMN delivery_slots text[] CHECK (cardinality(delivery_slots) > 0);
