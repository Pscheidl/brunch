ALTER TABLE products ADD COLUMN minimum_quantity integer NOT NULL DEFAULT 0
    CHECK (minimum_quantity BETWEEN 0 AND 99);

CREATE TABLE categories (
    id bigserial PRIMARY KEY,
    name text NOT NULL UNIQUE,
    version bigint NOT NULL DEFAULT 1
);

-- The existing empty category value denotes an unassigned product.
INSERT INTO categories(name) SELECT '' UNION SELECT DISTINCT category FROM products;
ALTER TABLE products ADD CONSTRAINT product_category_fk FOREIGN KEY (category)
    REFERENCES categories(name) ON UPDATE CASCADE ON DELETE SET DEFAULT;
CREATE INDEX products_category ON products(category);

-- A category cascade also changes the product seen by an open product editor.
CREATE FUNCTION bump_category_product_version() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    NEW.version := OLD.version + 1;
    NEW.updated_at := now();
    RETURN NEW;
END;
$$;
CREATE TRIGGER category_product_version BEFORE UPDATE OF category ON products
    FOR EACH ROW WHEN (OLD.category <> NEW.category AND OLD.version = NEW.version)
    EXECUTE FUNCTION bump_category_product_version();
