ALTER TABLE products DROP CONSTRAINT products_minimum_quantity_check;
ALTER TABLE products ADD CONSTRAINT products_minimum_quantity_check
    CHECK (minimum_quantity >= 0 AND (banquet OR minimum_quantity <= 99));

ALTER TABLE order_items DROP CONSTRAINT order_items_quantity_check;
ALTER TABLE order_items ADD CONSTRAINT order_items_quantity_check
    CHECK (quantity >= 1);
