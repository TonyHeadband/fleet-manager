-- Not idempotent: Store::open runs it once, gated on PRAGMA user_version. The git remote a product's internal repo
-- clones from and pushes to; NULL keeps the repo local-only.
ALTER TABLE products ADD COLUMN internal_url TEXT;
