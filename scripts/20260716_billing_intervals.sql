BEGIN;

ALTER TABLE billing_rates ADD COLUMN IF NOT EXISTS billing_interval_secs INTEGER;
ALTER TABLE billing_rates ADD COLUMN IF NOT EXISTS price_per_interval NUMERIC(20, 8);
UPDATE billing_rates SET billing_interval_secs = 60 WHERE billing_interval_secs IS NULL;
UPDATE billing_rates SET price_per_interval = rate_per_minute WHERE price_per_interval IS NULL;
ALTER TABLE billing_rates ALTER COLUMN billing_interval_secs SET DEFAULT 60;
ALTER TABLE billing_rates ALTER COLUMN billing_interval_secs SET NOT NULL;
ALTER TABLE billing_rates ALTER COLUMN price_per_interval SET DEFAULT 0;
ALTER TABLE billing_rates ALTER COLUMN price_per_interval SET NOT NULL;

ALTER TABLE billing_ledger ADD COLUMN IF NOT EXISTS billing_interval_secs INTEGER;
ALTER TABLE billing_ledger ADD COLUMN IF NOT EXISTS price_per_interval NUMERIC(20, 8);
UPDATE billing_ledger SET billing_interval_secs = 60 WHERE billing_interval_secs IS NULL;
UPDATE billing_ledger SET price_per_interval = rate_per_minute WHERE price_per_interval IS NULL;
ALTER TABLE billing_ledger ALTER COLUMN billing_interval_secs SET DEFAULT 60;
ALTER TABLE billing_ledger ALTER COLUMN billing_interval_secs SET NOT NULL;
ALTER TABLE billing_ledger ALTER COLUMN price_per_interval SET DEFAULT 0;
ALTER TABLE billing_ledger ALTER COLUMN price_per_interval SET NOT NULL;

COMMIT;
