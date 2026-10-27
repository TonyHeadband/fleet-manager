-- Not idempotent: Store::open runs it once, gated on PRAGMA user_version. A product's own settings, sparse: a row
-- exists only for a setting the product changes, so everything else keeps following the global value.
CREATE TABLE product_overrides (
    product TEXT NOT NULL REFERENCES products(id) ON DELETE CASCADE,
    section TEXT NOT NULL,
    key     TEXT NOT NULL,
    value   TEXT NOT NULL,
    PRIMARY KEY (product, section, key)
);
