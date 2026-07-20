BEGIN;

INSERT INTO billing_accounts (username, balance, currency, credit_limit)
VALUES
    ('sipp-business', 100.0, 'CNY', 0.0),
    ('2001', 100.0, 'CNY', 0.0)
ON CONFLICT (username) DO UPDATE
SET balance = GREATEST(billing_accounts.balance, 100.0), currency = EXCLUDED.currency;

-- HA1 = MD5("2001:vos-rs:Demo2001!"). Never store the clear-text password.
INSERT INTO sip_users (username, password)
VALUES ('2001', '94f094e3f3a7f0a8fd5c8977831aa9bd')
ON CONFLICT (username) DO UPDATE SET password = EXCLUDED.password;

INSERT INTO sip_gateways (
    id, host, port, transport, gateway_type, max_capacity, current_concurrent,
    caller_id_mode, max_concurrent, role, access_auth_mode, enabled
)
VALUES
    ('sipp-egress', '127.0.0.1', 5190, 'udp', 'peer', 20, 0, 'passthrough', 20, 'egress', 'none', TRUE),
    ('sipp-egress-fail', '127.0.0.1', 5191, 'udp', 'peer', 20, 0, 'passthrough', 20, 'egress', 'none', TRUE),
    ('sipp-access-pass', '127.0.0.1', 5164, 'udp', 'peer', 20, 0, 'passthrough', 20, 'access', 'ip_allowlist', TRUE),
    ('sipp-access-fixed', '127.0.0.1', 5165, 'udp', 'peer', 20, 0, 'fixed', 20, 'access', 'ip_allowlist', TRUE),
    ('sipp-access-pool', '127.0.0.1', 5166, 'udp', 'peer', 20, 0, 'virtual', 20, 'access', 'ip_allowlist', TRUE),
    ('sipp-access-fail', '127.0.0.1', 5167, 'udp', 'peer', 20, 0, 'fixed', 20, 'access', 'ip_allowlist', TRUE)
ON CONFLICT (id) DO UPDATE SET
    host = EXCLUDED.host,
    port = EXCLUDED.port,
    transport = EXCLUDED.transport,
    gateway_type = EXCLUDED.gateway_type,
    max_capacity = EXCLUDED.max_capacity,
    current_concurrent = 0,
    caller_id_mode = EXCLUDED.caller_id_mode,
    max_concurrent = EXCLUDED.max_concurrent,
    role = EXCLUDED.role,
    access_auth_mode = EXCLUDED.access_auth_mode,
    enabled = TRUE;

UPDATE sip_gateways
SET account_id = (SELECT id FROM billing_accounts WHERE username = 'sipp-business')
WHERE id LIKE 'sipp-access-%';

DELETE FROM trunk_ip_rules WHERE trunk_id LIKE 'sipp-access-%';
INSERT INTO trunk_ip_rules (trunk_id, cidr, source_port, transport, description, enabled)
VALUES
    ('sipp-access-pass', '127.0.0.1/32', 5164, 'udp', 'SIPp 严格透传接入', TRUE),
    ('sipp-access-fixed', '127.0.0.1/32', 5165, 'udp', 'SIPp 固定主叫接入', TRUE),
    ('sipp-access-pool', '127.0.0.1/32', 5166, 'udp', 'SIPp 号码池接入', TRUE),
    ('sipp-access-fail', '127.0.0.1/32', 5167, 'udp', 'SIPp 失败落地接入', TRUE);

DELETE FROM egress_endpoints WHERE trunk_id IN ('sipp-egress', 'sipp-egress-fail');
INSERT INTO egress_endpoints (trunk_id, host, port, transport, priority, enabled)
VALUES
    ('sipp-egress', '127.0.0.1', 5190, 'udp', 100, TRUE),
    ('sipp-egress-fail', '127.0.0.1', 5191, 'udp', 100, TRUE)
ON CONFLICT (trunk_id, host, port, transport) DO UPDATE
SET priority = EXCLUDED.priority, enabled = TRUE;

INSERT INTO sip_routes (id, prefix, priority, gateway_id, cost, weight)
VALUES
    ('sipp-route-main', '9', 10, 'sipp-egress', 0.05, 100),
    ('sipp-route-fail', '9', 20, 'sipp-egress-fail', 0.04, 100)
ON CONFLICT (id) DO UPDATE SET
    prefix = EXCLUDED.prefix,
    priority = EXCLUDED.priority,
    gateway_id = EXCLUDED.gateway_id,
    cost = EXCLUDED.cost,
    weight = EXCLUDED.weight;

INSERT INTO number_inventory (
    number, direction, max_concurrent, current_concurrent, status,
    owner_egress_trunk_id, gateway_id, updated_at
)
VALUES
    ('4008002001', 'both', 2, 0, 'assigned', 'sipp-egress', 'sipp-egress', NOW()),
    ('861380020001', 'outbound', 2, 0, 'assigned', 'sipp-egress', 'sipp-egress', NOW()),
    ('861380020002', 'outbound', 2, 0, 'assigned', 'sipp-egress', 'sipp-egress', NOW()),
    ('861380020003', 'outbound', 2, 0, 'assigned', 'sipp-egress-fail', 'sipp-egress-fail', NOW()),
    ('861380020101', 'outbound', 1, 0, 'assigned', 'sipp-egress', 'sipp-egress', NOW()),
    ('861380020102', 'outbound', 2, 0, 'assigned', 'sipp-egress-fail', 'sipp-egress-fail', NOW())
ON CONFLICT (number) DO UPDATE SET
    direction = EXCLUDED.direction,
    max_concurrent = EXCLUDED.max_concurrent,
    current_concurrent = 0,
    status = EXCLUDED.status,
    owner_egress_trunk_id = EXCLUDED.owner_egress_trunk_id,
    gateway_id = EXCLUDED.gateway_id,
    updated_at = NOW();

DELETE FROM number_allocations
WHERE number IN ('4008002001', '861380020001', '861380020002', '861380020003', '861380020101', '861380020102');
INSERT INTO number_allocations (number, source_type, source_id, enabled)
VALUES
    ('4008002001', 'extension', '2001', TRUE),
    ('861380020001', 'trunk', 'sipp-access-fixed', TRUE),
    ('861380020002', 'trunk', 'sipp-access-pass', TRUE),
    ('861380020003', 'trunk', 'sipp-access-fail', TRUE),
    ('861380020101', 'trunk', 'sipp-access-pool', TRUE),
    ('861380020102', 'trunk', 'sipp-access-pool', TRUE);

INSERT INTO caller_pools (
    id, owner_source_type, owner_source_id, virtual_alias, strategy,
    fallback_mode, enabled, updated_at
)
VALUES ('sipp-pool-access', 'trunk', 'sipp-access-pool', '95001', 'round_robin', 'reject', TRUE, NOW())
ON CONFLICT (id) DO UPDATE SET
    owner_source_type = EXCLUDED.owner_source_type,
    owner_source_id = EXCLUDED.owner_source_id,
    virtual_alias = EXCLUDED.virtual_alias,
    strategy = EXCLUDED.strategy,
    fallback_mode = EXCLUDED.fallback_mode,
    enabled = TRUE,
    updated_at = NOW();

INSERT INTO caller_pool_members (pool_id, number, priority, weight, max_concurrent, enabled)
VALUES
    ('sipp-pool-access', '861380020101', 100, 100, 1, TRUE),
    ('sipp-pool-access', '861380020102', 100, 100, 2, TRUE)
ON CONFLICT (pool_id, number) DO UPDATE SET
    priority = EXCLUDED.priority,
    weight = EXCLUDED.weight,
    max_concurrent = EXCLUDED.max_concurrent,
    enabled = TRUE;

INSERT INTO egress_groups (id, name, description, enabled, updated_at)
VALUES ('sipp-egress-group', 'SIPp 落地组', '覆盖号码池多落地归属与失败约束', TRUE, NOW())
ON CONFLICT (id) DO UPDATE SET
    name = EXCLUDED.name,
    description = EXCLUDED.description,
    enabled = TRUE,
    updated_at = NOW();

INSERT INTO egress_group_members (
    group_id, egress_trunk_id, destination_prefix, priority, weight, enabled
)
VALUES
    ('sipp-egress-group', 'sipp-egress', '9', 10, 100, TRUE),
    ('sipp-egress-group', 'sipp-egress-fail', '9', 20, 100, TRUE)
ON CONFLICT (group_id, egress_trunk_id, destination_prefix) DO UPDATE SET
    priority = EXCLUDED.priority,
    weight = EXCLUDED.weight,
    enabled = TRUE;

INSERT INTO source_outbound_policies (
    source_type, source_id, caller_mode, fixed_number, caller_pool_id,
    egress_mode, direct_egress_trunk_id, egress_group_id,
    fallback_mode, enabled, updated_at
)
VALUES
    ('trunk', 'sipp-access-pass', 'strict_passthrough', NULL, NULL, 'direct', 'sipp-egress', NULL, 'reject', TRUE, NOW()),
    ('trunk', 'sipp-access-fixed', 'fixed_number', '861380020001', NULL, 'direct', 'sipp-egress', NULL, 'reject', TRUE, NOW()),
    ('trunk', 'sipp-access-pool', 'virtual_pool', NULL, 'sipp-pool-access', 'group', NULL, 'sipp-egress-group', 'reject', TRUE, NOW()),
    ('trunk', 'sipp-access-fail', 'fixed_number', '861380020003', NULL, 'direct', 'sipp-egress-fail', NULL, 'reject', TRUE, NOW()),
    ('extension', '2001', 'fixed_number', '4008002001', NULL, 'direct', 'sipp-egress', NULL, 'reject', TRUE, NOW())
ON CONFLICT (source_type, source_id) DO UPDATE SET
    caller_mode = EXCLUDED.caller_mode,
    fixed_number = EXCLUDED.fixed_number,
    caller_pool_id = EXCLUDED.caller_pool_id,
    egress_mode = EXCLUDED.egress_mode,
    direct_egress_trunk_id = EXCLUDED.direct_egress_trunk_id,
    egress_group_id = EXCLUDED.egress_group_id,
    fallback_mode = EXCLUDED.fallback_mode,
    enabled = TRUE,
    updated_at = NOW();

INSERT INTO did_destinations (number, tenant_id, target_type, target_id, enabled, updated_at)
VALUES ('4008002001', NULL, 'extension', '2001', TRUE, NOW())
ON CONFLICT (number) DO UPDATE SET
    tenant_id = EXCLUDED.tenant_id,
    target_type = EXCLUDED.target_type,
    target_id = EXCLUDED.target_id,
    enabled = TRUE,
    updated_at = NOW();

INSERT INTO billing_rates (
    id, prefix, rate_per_minute, description, billing_interval_secs, price_per_interval
)
VALUES ('sipp-mobile-6s', '9', 0.5, 'SIPp 业务流：每 6 秒 0.05 元', 6, 0.05)
ON CONFLICT (id) DO UPDATE SET
    prefix = EXCLUDED.prefix,
    rate_per_minute = EXCLUDED.rate_per_minute,
    description = EXCLUDED.description,
    billing_interval_secs = EXCLUDED.billing_interval_secs,
    price_per_interval = EXCLUDED.price_per_interval;

COMMIT;
