ALTER TABLE billing_accounts
    ADD COLUMN IF NOT EXISTS credit_limit NUMERIC(20, 8) NOT NULL DEFAULT 0.0;

CREATE TABLE IF NOT EXISTS billing_credits (
    idempotency_key TEXT PRIMARY KEY,
    username TEXT NOT NULL,
    amount NUMERIC(20, 8) NOT NULL CHECK (amount > 0),
    balance_after NUMERIC(20, 8) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_billing_credits_username
    ON billing_credits (username, created_at DESC);
