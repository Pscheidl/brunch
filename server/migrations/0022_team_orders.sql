CREATE TABLE team_orders (
    id UUID PRIMARY KEY,
    owner_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 100),
    invite_token TEXT NOT NULL UNIQUE CHECK (length(invite_token) = 64),
    deadline TIMESTAMPTZ NOT NULL,
    closed_at TIMESTAMPTZ,
    order_id UUID UNIQUE REFERENCES orders(id),
    revision BIGINT NOT NULL DEFAULT 1,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX team_orders_owner ON team_orders(owner_id, created_at DESC);
CREATE TABLE team_contributions (
    id UUID PRIMARY KEY,
    team_id UUID NOT NULL REFERENCES team_orders(id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 80),
    items JSONB NOT NULL CHECK (jsonb_typeof(items) = 'array'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX team_contributions_team ON team_contributions(team_id, created_at);
