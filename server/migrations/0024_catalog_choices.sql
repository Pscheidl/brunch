ALTER TABLE products ADD COLUMN variants text[] NOT NULL DEFAULT '{}';
ALTER TABLE order_items ADD COLUMN variant text;
