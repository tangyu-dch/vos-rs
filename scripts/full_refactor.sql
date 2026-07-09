-- ============================================
-- 落地网关/对接网关/号码库存 全面重构
-- ============================================

-- 1. 删除旧视图
DROP VIEW IF EXISTS v_gateway_numbers CASCADE;
DROP VIEW IF EXISTS v_gateway_peer_links CASCADE;

-- 2. 删除旧关联表
DROP TABLE IF EXISTS gateway_number_assignments CASCADE;
DROP TABLE IF EXISTS gateway_peer_links CASCADE;

-- 3. 重建 sip_gateways（保留已有数据）
-- 先备份
CREATE TEMPORARY TABLE _backup_gateways AS SELECT * FROM sip_gateways;

DROP TABLE IF EXISTS sip_routes CASCADE;
DROP TABLE IF EXISTS sip_gateways CASCADE;

CREATE TABLE sip_gateways (
    id VARCHAR(50) PRIMARY KEY,
    host VARCHAR(255) NOT NULL,
    port INTEGER DEFAULT 5060,
    transport VARCHAR(10) NOT NULL DEFAULT 'udp',
    gateway_type VARCHAR(20) NOT NULL DEFAULT 'peer',
    max_capacity INTEGER DEFAULT 100,
    current_concurrent INTEGER DEFAULT 0,
    caller_id_mode VARCHAR(20) DEFAULT 'passthrough',
    virtual_caller VARCHAR(50) DEFAULT '',
    prefix_rules TEXT DEFAULT '',
    supports_registration BOOLEAN DEFAULT false,
    reg_auth_type VARCHAR(10) DEFAULT 'ip',
    reg_username VARCHAR(50) DEFAULT '',
    reg_password VARCHAR(100) DEFAULT '',
    parent_gateway_id VARCHAR(50) DEFAULT '',
    account_id BIGINT,
    enabled BOOLEAN DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

INSERT INTO sip_gateways (id, host, port, transport, max_capacity, gateway_type, prefix_rules, supports_registration, parent_gateway_id, created_at)
SELECT id, host, port, transport,
       COALESCE(max_capacity, 100),
       COALESCE(gateway_type, 'peer'),
       COALESCE(prefix_rules, ''),
       COALESCE(supports_registration, false),
       COALESCE(parent_gateway_id, ''),
       created_at
FROM _backup_gateways;

DROP TABLE IF EXISTS _backup_gateways;

-- 4. 重建 number_inventory
CREATE TEMPORARY TABLE _backup_numbers AS SELECT * FROM number_inventory;
DROP TABLE IF EXISTS number_inventory CASCADE;

CREATE TABLE number_inventory (
    number VARCHAR(20) PRIMARY KEY,
    username VARCHAR(50),
    gateway_id VARCHAR(50),
    direction VARCHAR(20) DEFAULT 'bidirectional',
    max_concurrent INTEGER DEFAULT 10,
    current_concurrent INTEGER DEFAULT 0,
    status VARCHAR(20) NOT NULL DEFAULT 'available',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

INSERT INTO number_inventory (number, username, status, created_at)
SELECT number, username, status, created_at FROM _backup_numbers;

DROP TABLE IF EXISTS _backup_numbers;

-- 5. 创建 gateway_number_assignments
CREATE TABLE gateway_number_assignments (
    id BIGSERIAL PRIMARY KEY,
    gateway_id VARCHAR(50) NOT NULL REFERENCES sip_gateways(id) ON DELETE CASCADE,
    number VARCHAR(20) NOT NULL REFERENCES number_inventory(number) ON DELETE CASCADE,
    direction VARCHAR(20) DEFAULT 'both',
    max_concurrent INTEGER DEFAULT 10,
    enabled BOOLEAN DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(gateway_id, number)
);
CREATE INDEX idx_gna_gateway ON gateway_number_assignments(gateway_id);
CREATE INDEX idx_gna_number ON gateway_number_assignments(number);

-- 6. 创建 gateway_peer_links
CREATE TABLE gateway_peer_links (
    id BIGSERIAL PRIMARY KEY,
    gateway_id VARCHAR(50) NOT NULL REFERENCES sip_gateways(id) ON DELETE CASCADE,
    peer_gateway_id VARCHAR(50) NOT NULL REFERENCES sip_gateways(id) ON DELETE CASCADE,
    enabled BOOLEAN DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(gateway_id, peer_gateway_id)
);
CREATE INDEX idx_gpl_gateway ON gateway_peer_links(gateway_id);

-- 7. 重建路由表
CREATE TABLE sip_routes (
    id VARCHAR(50) PRIMARY KEY,
    prefix TEXT NOT NULL,
    priority INTEGER NOT NULL DEFAULT 100,
    gateway_id VARCHAR(50) NOT NULL REFERENCES sip_gateways(id) ON DELETE CASCADE,
    cost DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    time_start TEXT,
    time_end TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_sip_routes_prefix ON sip_routes(prefix);

-- 8. 视图
CREATE OR REPLACE VIEW v_gateway_numbers AS
SELECT gna.gateway_id, g.host AS gateway_host, g.gateway_type, g.caller_id_mode,
       gna.number, ni.username, ni.status AS number_status, ni.direction AS number_direction,
       gna.direction AS assignment_direction, gna.max_concurrent, gna.enabled
FROM gateway_number_assignments gna
JOIN sip_gateways g ON g.id = gna.gateway_id
LEFT JOIN number_inventory ni ON ni.number = gna.number
WHERE gna.enabled = true;

CREATE OR REPLACE VIEW v_gateway_peer_links AS
SELECT gpl.gateway_id, g1.host AS gateway_host,
       gpl.peer_gateway_id, g2.host AS peer_host, g2.port AS peer_port,
       g2.transport AS peer_transport, gpl.enabled
FROM gateway_peer_links gpl
JOIN sip_gateways g1 ON g1.id = gpl.gateway_id
JOIN sip_gateways g2 ON g2.id = gpl.peer_gateway_id
WHERE gpl.enabled = true;
