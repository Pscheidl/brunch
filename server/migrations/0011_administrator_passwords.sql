-- Administrator accounts are managed in the database, never from environment.
ALTER TABLE users ADD COLUMN admin_password_hash TEXT;
ALTER TABLE sessions ADD COLUMN admin_authenticated BOOLEAN NOT NULL DEFAULT FALSE;

-- Revoke every legacy administrator session. Preserve the user/customer data.
DELETE FROM sessions WHERE user_id IN (SELECT id FROM users WHERE is_admin);
UPDATE users SET is_admin=FALSE WHERE is_admin;
ALTER TABLE users ADD CONSTRAINT administrator_requires_password
    CHECK (NOT is_admin OR admin_password_hash IS NOT NULL);
