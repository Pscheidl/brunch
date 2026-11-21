CREATE TABLE mail_broadcasts (
    id UUID PRIMARY KEY,
    created_by UUID NOT NULL REFERENCES users(id),
    subject TEXT NOT NULL,
    body TEXT NOT NULL,
    recipient_count BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

ALTER TABLE outbox ADD COLUMN broadcast_id UUID REFERENCES mail_broadcasts(id);
CREATE UNIQUE INDEX outbox_broadcast_recipient ON outbox(broadcast_id, to_email)
    WHERE broadcast_id IS NOT NULL;

CREATE VIEW order_email_recipients AS
    SELECT DISTINCT lower(trim(email)) AS email FROM orders WHERE trim(email) <> '';
