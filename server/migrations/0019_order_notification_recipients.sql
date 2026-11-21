ALTER TABLE shop_settings
    ADD COLUMN additional_order_emails text[] NOT NULL DEFAULT '{}';
