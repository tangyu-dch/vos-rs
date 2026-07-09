-- ============================================
-- 号码库存与网关管理全面升级
-- 参考昆石 VOS 设计
-- ============================================

-- 1. 增强 number_inventory 表
ALTER TABLE number_inventory ADD COLUMN IF NOT EXISTS gateway_id VARCHAR(50) DEFAULT '';
ALTER TABLE number_inventory ADD COLUMN IF NOT EXISTS direction VARCHAR(20) DEFAULT 'bidirectional';
ALTER TABLE number_inventory ADD COLUMN IF NOT EXISTS max_concurrent INT DEFAULT 10;
ALTER TABLE number_inventory ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ DEFAULT NOW();

COMMENT ON COLUMN number_inventory.gateway_id IS '归属的对接网关ID';
COMMENT ON COLUMN number_inventory.direction IS '号码方向: inbound/outbound/bidirectional';
COMMENT ON COLUMN number_inventory.max_concurrent IS '号码级并发上限';

CREATE INDEX IF NOT EXISTS idx_number_inventory_gateway ON number_inventory(gateway_id);
CREATE INDEX IF NOT EXISTS idx_number_inventory_status ON number_inventory(status);

-- 2. 增强 sip_gateways 表
ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS caller_id_mode VARCHAR(20) DEFAULT 'passthrough';
ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS virtual_caller VARCHAR(50) DEFAULT '';
ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS current_concurrent INT DEFAULT 0;

COMMENT ON COLUMN sip_gateways.caller_id_mode IS '主叫处理: passthrough(透传)/virtual(虚拟主叫)/random(随机选号)';
COMMENT ON COLUMN sip_gateways.virtual_caller IS '虚拟主叫号码（caller_id_mode=virtual 时使用）';
COMMENT ON COLUMN sip_gateways.current_concurrent IS '当前并发数（运行时更新）';

-- 3. 号码分配表
CREATE TABLE IF NOT EXISTS gateway_number_assignments (
    id BIGSERIAL PRIMARY KEY,
    gateway_id VARCHAR(50) NOT NULL,
    number VARCHAR(20) NOT NULL,
    direction VARCHAR(20) DEFAULT 'both',
    max_concurrent INT DEFAULT 10,
    enabled BOOLEAN DEFAULT true,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    FOREIGN KEY (gateway_id) REFERENCES sip_gateways(id) ON DELETE CASCADE,
    FOREIGN KEY (number) REFERENCES number_inventory(number) ON DELETE CASCADE,
    UNIQUE(gateway_id, number)
);

COMMENT ON TABLE gateway_number_assignments IS '落地网关号码分配表';
COMMENT ON COLUMN gateway_number_assignments.direction IS '允许方向: inbound/outbound/both';

-- 4. 落地网关 ↔ 对接网关关联表
CREATE TABLE IF NOT EXISTS gateway_peer_links (
    id BIGSERIAL PRIMARY KEY,
    gateway_id VARCHAR(50) NOT NULL,
    peer_gateway_id VARCHAR(50) NOT NULL,
    enabled BOOLEAN DEFAULT true,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    FOREIGN KEY (gateway_id) REFERENCES sip_gateways(id) ON DELETE CASCADE,
    FOREIGN KEY (peer_gateway_id) REFERENCES sip_gateways(id) ON DELETE CASCADE,
    UNIQUE(gateway_id, peer_gateway_id)
);

COMMENT ON TABLE gateway_peer_links IS '落地网关与对接网关关联表';

-- 5. 号码分配视图（方便查询）
CREATE OR REPLACE VIEW v_gateway_numbers AS
SELECT 
    gna.gateway_id,
    g.id AS gateway_name,
    g.host AS gateway_host,
    g.gateway_type,
    g.caller_id_mode,
    g.virtual_caller,
    gna.number,
    ni.username,
    ni.status AS number_status,
    ni.direction AS number_direction,
    gna.direction AS assignment_direction,
    gna.max_concurrent,
    gna.enabled
FROM gateway_number_assignments gna
JOIN sip_gateways g ON g.id = gna.gateway_id
LEFT JOIN number_inventory ni ON ni.number = gna.number
WHERE gna.enabled = true;

-- 6. 落地网关 ↔ 对接网关关联视图
CREATE OR REPLACE VIEW v_gateway_peer_links AS
SELECT 
    gpl.gateway_id,
    g1.id AS gateway_name,
    g1.host AS gateway_host,
    gpl.peer_gateway_id,
    g2.id AS peer_name,
    g2.host AS peer_host,
    g2.port AS peer_port,
    g2.transport AS peer_transport,
    g2.prefix_rules AS peer_prefix_rules,
    gpl.enabled
FROM gateway_peer_links gpl
JOIN sip_gateways g1 ON g1.id = gpl.gateway_id
JOIN sip_gateways g2 ON g2.id = gpl.peer_gateway_id
WHERE gpl.enabled = true;
