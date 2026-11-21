-- Backfill customer accounts without changing existing roles, passwords, or sessions.
INSERT INTO users(id,email)
SELECT gen_random_uuid(), email
FROM (SELECT DISTINCT lower(btrim(email)) AS email FROM orders) AS customer_emails
ON CONFLICT(email) DO NOTHING;

-- Ownership follows the order email. Preserve the order snapshot and audit actor.
UPDATE orders AS o
SET user_id=u.id
FROM users AS u
WHERE u.email=lower(btrim(o.email)) AND o.user_id IS DISTINCT FROM u.id;
