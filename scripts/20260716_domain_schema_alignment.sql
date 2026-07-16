-- vos-rs 网关、号码、路由领域规范增量迁移。
-- 可重复执行；不删除、不重建业务表，也不覆盖已有配置数据。
BEGIN;

ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS gateway_type VARCHAR(20) NOT NULL DEFAULT 'peer';
ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS prefix_rules TEXT NOT NULL DEFAULT '';
ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS supports_registration BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS reg_auth_type VARCHAR(20) NOT NULL DEFAULT 'none';
ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS reg_username TEXT NOT NULL DEFAULT '';
ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS reg_password TEXT NOT NULL DEFAULT '';
ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS parent_gateway_id TEXT;
ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS caller_id_mode VARCHAR(20) NOT NULL DEFAULT 'passthrough';
ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS virtual_caller TEXT NOT NULL DEFAULT '';
ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS current_concurrent INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS max_concurrent INTEGER NOT NULL DEFAULT 100;
ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS account_id BIGINT;
ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS enabled BOOLEAN NOT NULL DEFAULT TRUE;

DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'public'
          AND table_name = 'sip_gateways'
          AND column_name = 'account_id'
          AND data_type = 'integer'
    ) THEN
        ALTER TABLE sip_gateways DROP CONSTRAINT IF EXISTS fk_gateway_account;
        ALTER TABLE sip_gateways ALTER COLUMN account_id TYPE BIGINT USING account_id::BIGINT;
    END IF;
END $$;

ALTER TABLE sip_routes ADD COLUMN IF NOT EXISTS cost DOUBLE PRECISION NOT NULL DEFAULT 0.0;
ALTER TABLE sip_routes ADD COLUMN IF NOT EXISTS weight INTEGER NOT NULL DEFAULT 100;
ALTER TABLE sip_routes ADD COLUMN IF NOT EXISTS time_start TEXT;
ALTER TABLE sip_routes ADD COLUMN IF NOT EXISTS time_end TEXT;

ALTER TABLE number_inventory ADD COLUMN IF NOT EXISTS gateway_id TEXT;
ALTER TABLE number_inventory ADD COLUMN IF NOT EXISTS direction VARCHAR(20) NOT NULL DEFAULT 'bidirectional';
ALTER TABLE number_inventory ADD COLUMN IF NOT EXISTS max_concurrent INTEGER NOT NULL DEFAULT 10;
ALTER TABLE number_inventory ADD COLUMN IF NOT EXISTS current_concurrent INTEGER NOT NULL DEFAULT 0;
ALTER TABLE number_inventory ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ NOT NULL DEFAULT now();

ALTER TABLE billing_accounts ADD COLUMN IF NOT EXISTS id BIGSERIAL;
CREATE UNIQUE INDEX IF NOT EXISTS idx_billing_accounts_id ON billing_accounts (id);

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'fk_gateway_account') THEN
        ALTER TABLE sip_gateways
            ADD CONSTRAINT fk_gateway_account
            FOREIGN KEY (account_id) REFERENCES billing_accounts(id)
            ON DELETE SET NULL NOT VALID;
    END IF;
END $$;

CREATE TABLE IF NOT EXISTS gateway_number_assignments (
    id BIGSERIAL PRIMARY KEY,
    gateway_id TEXT NOT NULL REFERENCES sip_gateways(id) ON DELETE CASCADE,
    number TEXT NOT NULL REFERENCES number_inventory(number) ON DELETE CASCADE,
    direction VARCHAR(20) NOT NULL DEFAULT 'both',
    max_concurrent INTEGER NOT NULL DEFAULT 10,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (gateway_id, number)
);

CREATE TABLE IF NOT EXISTS gateway_peer_links (
    id BIGSERIAL PRIMARY KEY,
    gateway_id TEXT NOT NULL REFERENCES sip_gateways(id) ON DELETE CASCADE,
    peer_gateway_id TEXT NOT NULL REFERENCES sip_gateways(id) ON DELETE CASCADE,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (gateway_id, peer_gateway_id),
    CHECK (gateway_id <> peer_gateway_id)
);

CREATE INDEX IF NOT EXISTS idx_sip_gateways_type ON sip_gateways (gateway_type);
CREATE INDEX IF NOT EXISTS idx_sip_gateways_parent ON sip_gateways (parent_gateway_id);
CREATE INDEX IF NOT EXISTS idx_sip_gateways_account ON sip_gateways (account_id);
CREATE INDEX IF NOT EXISTS idx_sip_gateways_enabled ON sip_gateways (enabled);
CREATE INDEX IF NOT EXISTS idx_sip_routes_priority_id ON sip_routes (priority, id);
CREATE INDEX IF NOT EXISTS idx_sip_routes_prefix ON sip_routes (prefix);
CREATE INDEX IF NOT EXISTS idx_sip_routes_gateway ON sip_routes (gateway_id);
CREATE INDEX IF NOT EXISTS idx_number_inventory_gateway ON number_inventory (gateway_id);
CREATE INDEX IF NOT EXISTS idx_number_inventory_status ON number_inventory (status);
CREATE INDEX IF NOT EXISTS idx_number_inventory_username ON number_inventory (username);
CREATE INDEX IF NOT EXISTS idx_gna_gateway ON gateway_number_assignments (gateway_id);
CREATE INDEX IF NOT EXISTS idx_gna_number ON gateway_number_assignments (number);
CREATE INDEX IF NOT EXISTS idx_gpl_gateway ON gateway_peer_links (gateway_id);
CREATE INDEX IF NOT EXISTS idx_gpl_peer_gateway ON gateway_peer_links (peer_gateway_id);

COMMIT;
