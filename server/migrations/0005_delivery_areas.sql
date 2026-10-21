CREATE TABLE delivery_areas (
    id BIGSERIAL PRIMARY KEY,
    postal_code TEXT NOT NULL UNIQUE CHECK (postal_code ~ '^[0-9]{5}$'),
    label TEXT NOT NULL DEFAULT '' CHECK (char_length(label) <= 100),
    active BOOLEAN NOT NULL DEFAULT TRUE,
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0)
);
