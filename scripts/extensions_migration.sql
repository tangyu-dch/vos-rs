-- 分机表（注册接入）
CREATE TABLE IF NOT EXISTS sip_extensions (
    id BIGSERIAL PRIMARY KEY,
    gateway_id TEXT NOT NULL REFERENCES sip_gateways(id) ON DELETE CASCADE,
    extension_number VARCHAR(50) NOT NULL,
    extension_password VARCHAR(200) NOT NULL,
    registration_method VARCHAR(20) NOT NULL DEFAULT 'digest',
    is_auto_created BOOLEAN DEFAULT false,
    enabled BOOLEAN DEFAULT true,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_sip_extensions_gateway ON sip_extensions(gateway_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_sip_extensions_number ON sip_extensions(extension_number);
