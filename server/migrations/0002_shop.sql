CREATE TABLE products (
    id BIGSERIAL PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    category TEXT NOT NULL DEFAULT '',
    tag TEXT NOT NULL DEFAULT '',
    weight TEXT NOT NULL DEFAULT '',
    price_cents BIGINT NOT NULL CHECK (price_cents BETWEEN 0 AND 10000000),
    vegan BOOLEAN NOT NULL DEFAULT FALSE,
    ingredients TEXT NOT NULL DEFAULT '',
    allergens TEXT NOT NULL DEFAULT '',
    image TEXT NOT NULL DEFAULT '',
    active BOOLEAN NOT NULL DEFAULT TRUE,
    stock INTEGER CHECK (stock BETWEEN 0 AND 1000000),
    version BIGINT NOT NULL DEFAULT 1,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE product_prices (
    id BIGSERIAL PRIMARY KEY,
    product_id BIGINT NOT NULL REFERENCES products(id),
    price_cents BIGINT NOT NULL CHECK (price_cents BETWEEN 0 AND 10000000),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    changed_by UUID REFERENCES users(id)
);
CREATE INDEX product_prices_product ON product_prices(product_id, id DESC);

CREATE TABLE shop_settings (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    delivery_fee_cents BIGINT NOT NULL CHECK (delivery_fee_cents BETWEEN 0 AND 10000000),
    free_delivery_from_cents BIGINT CHECK (free_delivery_from_cents BETWEEN 0 AND 1000000000),
    order_email TEXT NOT NULL,
    version BIGINT NOT NULL DEFAULT 1,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE SEQUENCE order_number_seq;
CREATE TABLE orders (
    id UUID PRIMARY KEY,
    number TEXT NOT NULL UNIQUE DEFAULT ('BR-' || lpad(nextval('order_number_seq')::text, 8, '0')),
    idempotency_key UUID NOT NULL UNIQUE,
    request_payload JSONB NOT NULL,
    user_id UUID REFERENCES users(id),
    email TEXT NOT NULL,
    name TEXT NOT NULL DEFAULT '',
    phone TEXT NOT NULL DEFAULT '',
    organization TEXT NOT NULL DEFAULT '',
    street TEXT NOT NULL,
    city TEXT NOT NULL,
    postal_code TEXT NOT NULL,
    notes TEXT NOT NULL DEFAULT '',
    delivery_date DATE NOT NULL,
    delivery_slot TEXT NOT NULL,
    payment_method TEXT NOT NULL CHECK (payment_method IN ('card_on_delivery', 'cash_on_delivery')),
    status TEXT NOT NULL DEFAULT 'new' CHECK (status IN ('new','confirmed','preparing','out_for_delivery','delivered','cancelled')),
    payment_status TEXT NOT NULL DEFAULT 'unpaid' CHECK (payment_status IN ('unpaid','paid')),
    subtotal_cents BIGINT NOT NULL CHECK (subtotal_cents BETWEEN 0 AND 1000000000),
    delivery_fee_cents BIGINT NOT NULL CHECK (delivery_fee_cents BETWEEN 0 AND 10000000),
    total_cents BIGINT NOT NULL CHECK (total_cents = subtotal_cents + delivery_fee_cents),
    stock_restored BOOLEAN NOT NULL DEFAULT FALSE,
    version BIGINT NOT NULL DEFAULT 1,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (status <> 'cancelled' OR payment_status = 'unpaid')
);
CREATE INDEX orders_user ON orders(user_id, created_at DESC);
CREATE INDEX orders_status ON orders(status, created_at DESC);
CREATE INDEX orders_created ON orders(created_at DESC);

CREATE TABLE order_items (
    id BIGSERIAL PRIMARY KEY,
    order_id UUID NOT NULL REFERENCES orders(id),
    product_id BIGINT NOT NULL REFERENCES products(id),
    name TEXT NOT NULL,
    quantity INTEGER NOT NULL CHECK (quantity BETWEEN 1 AND 99),
    unit_price_cents BIGINT NOT NULL CHECK (unit_price_cents BETWEEN 0 AND 10000000),
    total_cents BIGINT NOT NULL CHECK (total_cents = quantity * unit_price_cents),
    tracked_stock BOOLEAN NOT NULL,
    UNIQUE (order_id, product_id)
);
CREATE INDEX order_items_order ON order_items(order_id);

CREATE TABLE order_history (
    id BIGSERIAL PRIMARY KEY,
    order_id UUID NOT NULL REFERENCES orders(id),
    status TEXT NOT NULL,
    payment_status TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    changed_by UUID REFERENCES users(id)
);
CREATE INDEX order_history_order ON order_history(order_id, id);
