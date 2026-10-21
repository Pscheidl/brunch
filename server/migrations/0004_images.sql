CREATE TABLE images (
    id UUID PRIMARY KEY,
    content_type TEXT NOT NULL CHECK (content_type IN ('image/jpeg', 'image/png')),
    data BYTEA NOT NULL CHECK (octet_length(data) BETWEEN 1 AND 8388608),
    created_by UUID NOT NULL REFERENCES users(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
