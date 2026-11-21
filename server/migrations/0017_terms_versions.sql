CREATE TABLE terms_versions (
    version bigint PRIMARY KEY CHECK (version > 0),
    title text NOT NULL CHECK (char_length(title) BETWEEN 1 AND 160),
    content text NOT NULL CHECK (char_length(content) BETWEEN 1 AND 100000),
    created_at timestamptz NOT NULL DEFAULT now(),
    created_by uuid REFERENCES users(id)
);

CREATE TABLE terms_publication (
    id integer PRIMARY KEY CHECK (id = 1),
    version bigint NOT NULL REFERENCES terms_versions(version)
);

ALTER TABLE orders ADD COLUMN terms_version bigint REFERENCES terms_versions(version);
