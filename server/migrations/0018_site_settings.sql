CREATE TABLE site_settings (
    id integer PRIMARY KEY CHECK (id = 1),
    hero_image text NOT NULL DEFAULT '' CHECK (
        hero_image = '' OR hero_image ~ '^/api/images/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
    ),
    hero_image_alt text NOT NULL DEFAULT 'Kváskový sendvič, jogurt s granolou a hummus se zeleninou na prostřeném stole'
        CHECK (char_length(btrim(hero_image_alt)) BETWEEN 1 AND 500),
    version bigint NOT NULL DEFAULT 1 CHECK (version > 0),
    updated_at timestamptz NOT NULL DEFAULT now()
);

INSERT INTO site_settings (id) VALUES (1);
