-- Historical orders retain NULL because their original connection is unknown.
ALTER TABLE orders ADD COLUMN created_from_ip inet;
