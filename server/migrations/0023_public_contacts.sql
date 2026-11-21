ALTER TABLE site_settings
    ADD COLUMN public_contact_email text NOT NULL DEFAULT '',
    ADD COLUMN public_contact_phone text NOT NULL DEFAULT '',
    ADD COLUMN seller_name text NOT NULL DEFAULT '',
    ADD COLUMN seller_address text NOT NULL DEFAULT '',
    ADD COLUMN seller_ico text NOT NULL DEFAULT '',
    ADD COLUMN delivery_area_label text NOT NULL DEFAULT '';
