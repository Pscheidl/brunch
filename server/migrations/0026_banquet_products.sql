ALTER TABLE products ADD COLUMN banquet boolean NOT NULL DEFAULT false;
ALTER TABLE products ADD CONSTRAINT banquet_product_minimum
    CHECK (NOT banquet OR minimum_quantity >= 1);

-- Existing products keep their classification and individual minimums.
ALTER TABLE shop_settings ADD COLUMN banquet_minimum_quantity integer NOT NULL DEFAULT 30
    CHECK (banquet_minimum_quantity BETWEEN 1 AND 4950);
