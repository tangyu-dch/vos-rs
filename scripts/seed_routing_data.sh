#!/usr/bin/env bash
# ====================================================================
# VOS-RS 中继路由测试数据种子脚本 (精准 SQL 语法与表结构版)
# 创建完整的测试数据，各实体之间逻辑关联正确
# ====================================================================

set -euo pipefail

DB_URL="${VOS_RS_DATABASE_URL:-postgres://tangyu@127.0.0.1:5432/vos_rs}"

echo "===================================================="
echo " [VOS-RS] 中继路由测试数据种子脚本"
echo "===================================================="
echo ""

psql "$DB_URL" <<'EOSQL'

-- 1. 对接账户 (Access Accounts)
INSERT INTO billing_accounts (username, account_type, balance, credit_limit, price_per_interval, billing_interval_secs, enabled)
VALUES
  ('acc-customer-alpha', 'access', 50000.00, 10000.00, 0.08, 60, TRUE),
  ('acc-customer-beta',  'access', 30000.00, 5000.00,  0.06, 60, TRUE)
ON CONFLICT (username) DO UPDATE SET
  balance = EXCLUDED.balance,
  credit_limit = EXCLUDED.credit_limit,
  price_per_interval = EXCLUDED.price_per_interval;

-- 2. 落地账户 (Egress Accounts)
INSERT INTO billing_accounts (username, account_type, balance, credit_limit, price_per_interval, billing_interval_secs, enabled)
VALUES
  ('cost-china-mobile',  'egress', 100000.00, 50000.00, 0.003, 6, TRUE),
  ('cost-china-unicom',  'egress', 80000.00,  30000.00, 0.0025, 6, TRUE),
  ('cost-china-telecom', 'egress', 60000.00,  20000.00, 0.0035, 6, TRUE)
ON CONFLICT (username) DO UPDATE SET
  balance = EXCLUDED.balance,
  credit_limit = EXCLUDED.credit_limit,
  price_per_interval = EXCLUDED.price_per_interval;

-- 3. 落地中继 (Egress Trunks)
INSERT INTO sip_gateways (id, host, port, transport, role, max_capacity, max_concurrent, account_id, enabled)
VALUES
  ('egress-cmcc-bj', '10.0.1.10', 5060, 'udp', 'egress', 200, 200,
    (SELECT id FROM billing_accounts WHERE username = 'cost-china-mobile' LIMIT 1), TRUE),
  ('egress-cmcc-sh', '10.0.1.11', 5060, 'udp', 'egress', 150, 150,
    (SELECT id FROM billing_accounts WHERE username = 'cost-china-mobile' LIMIT 1), TRUE),
  ('egress-cucc-bj', '10.0.2.10', 5060, 'udp', 'egress', 180, 180,
    (SELECT id FROM billing_accounts WHERE username = 'cost-china-unicom' LIMIT 1), TRUE),
  ('egress-ctcc-gz', '10.0.3.10', 5060, 'udp', 'egress', 120, 120,
    (SELECT id FROM billing_accounts WHERE username = 'cost-china-telecom' LIMIT 1), TRUE)
ON CONFLICT (id) DO UPDATE SET
  host = EXCLUDED.host,
  account_id = EXCLUDED.account_id,
  enabled = EXCLUDED.enabled;

-- 4. 接入中继 (Access Trunks)
INSERT INTO sip_gateways (id, host, port, transport, role, access_auth_mode, max_capacity, max_concurrent, account_id, enabled)
VALUES
  ('access-alpha', '192.168.1.100', 5060, 'udp', 'access', 'ip_allowlist', 100, 100,
    (SELECT id FROM billing_accounts WHERE username = 'acc-customer-alpha' LIMIT 1), TRUE),
  ('access-beta', '192.168.1.200;192.168.1.201', 5060, 'udp', 'access', 'ip_allowlist', 80, 80,
    (SELECT id FROM billing_accounts WHERE username = 'acc-customer-beta' LIMIT 1), TRUE)
ON CONFLICT (id) DO UPDATE SET
  host = EXCLUDED.host,
  account_id = EXCLUDED.account_id,
  enabled = EXCLUDED.enabled;

-- 5. 路由策略 (Route Rules)
INSERT INTO sip_routes (id, prefix, priority, gateway_id, cost, weight)
VALUES
  ('route-mobile-prefix',   '138', 10,  'egress-cmcc-bj', 0.030, 100),
  ('route-unicom-prefix',   '186', 10,  'egress-cucc-bj', 0.025, 100),
  ('route-telecom-prefix',  '189', 10,  'egress-ctcc-gz', 0.035, 100),
  ('route-default-fallback', '',   999, 'egress-cmcc-bj', 0.050, 100)
ON CONFLICT (id) DO UPDATE SET
  prefix = EXCLUDED.prefix,
  gateway_id = EXCLUDED.gateway_id,
  cost = EXCLUDED.cost;

-- 6. 落地分组 (Egress Groups)
INSERT INTO egress_groups (id, name, description, enabled)
VALUES
  ('group-cmcc',         '移动线路组',   '中国移动北京(主)+上海(备)双节点高可用落地分组', TRUE),
  ('group-all-carriers', '全网落地组',   '三大运营商按权重分流：移动50%/联通30%/电信20%', TRUE)
ON CONFLICT (id) DO UPDATE SET
  name = EXCLUDED.name,
  description = EXCLUDED.description;

-- 7. 落地分组成员 (Egress Group Members)
INSERT INTO egress_group_members (group_id, egress_trunk_id, destination_prefix, priority, weight, enabled)
VALUES
  ('group-cmcc', 'egress-cmcc-bj', '', 10, 100, TRUE),
  ('group-cmcc', 'egress-cmcc-sh', '', 20, 100, TRUE),
  ('group-all-carriers', 'egress-cmcc-bj', '', 10, 50,  TRUE),
  ('group-all-carriers', 'egress-cucc-bj', '', 10, 30,  TRUE),
  ('group-all-carriers', 'egress-ctcc-gz', '', 10, 20,  TRUE)
ON CONFLICT (group_id, egress_trunk_id, destination_prefix) DO UPDATE SET
  priority = EXCLUDED.priority,
  weight = EXCLUDED.weight;

-- 8. 真实号码 (Number Inventory)
INSERT INTO number_inventory (number, owner_egress_trunk_id, status, direction, max_concurrent)
VALUES
  ('13800001001', 'egress-cmcc-bj', 'available', 'bidirectional', 5),
  ('13800001002', 'egress-cmcc-bj', 'available', 'bidirectional', 5),
  ('13800001003', 'egress-cmcc-sh', 'available', 'bidirectional', 5),
  ('18600002001', 'egress-cucc-bj', 'available', 'bidirectional', 5),
  ('18900003001', 'egress-ctcc-gz', 'available', 'bidirectional', 5)
ON CONFLICT (number) DO UPDATE SET
  owner_egress_trunk_id = EXCLUDED.owner_egress_trunk_id,
  status = EXCLUDED.status;

-- 9. 号码池组 (Caller Pools)
INSERT INTO caller_pools (id, owner_source_type, owner_source_id, virtual_alias, strategy, fallback_mode, enabled)
VALUES
  ('pool-alpha-mobile', 'trunk', 'access-alpha', '400-888-0001', 'round_robin', 'reject', TRUE),
  ('pool-beta-mixed',   'trunk', 'access-beta',  '400-999-0002', 'random',      'reject', TRUE)
ON CONFLICT (id) DO UPDATE SET
  virtual_alias = EXCLUDED.virtual_alias,
  strategy = EXCLUDED.strategy;

-- 10. 号码池成员 (Caller Pool Members)
INSERT INTO caller_pool_members (pool_id, number, priority, weight, max_concurrent, enabled)
VALUES
  ('pool-alpha-mobile', '13800001001', 10,  100, 5, TRUE),
  ('pool-alpha-mobile', '13800001002', 20,  100, 5, TRUE),
  ('pool-alpha-mobile', '13800001003', 30,  100, 5, TRUE),
  ('pool-beta-mixed',   '18600002001', 10,  60,  5, TRUE),
  ('pool-beta-mixed',   '18900003001', 10,  40,  5, TRUE)
ON CONFLICT (pool_id, number) DO UPDATE SET
  priority = EXCLUDED.priority,
  weight = EXCLUDED.weight;

-- 11. 呼出主叫与落地关联策略 (Source Outbound Policies)
-- 接入中继 access-alpha 使用号码池 pool-alpha-mobile，落地走移动分组 group-cmcc
INSERT INTO source_outbound_policies (source_type, source_id, caller_mode, caller_pool_id, egress_mode, egress_group_id, fallback_mode, enabled)
VALUES
  ('trunk', 'access-alpha', 'virtual_pool', 'pool-alpha-mobile', 'group', 'group-cmcc', 'reject', TRUE),
  ('trunk', 'access-beta',  'virtual_pool', 'pool-beta-mixed',   'group', 'group-all-carriers', 'reject', TRUE)
ON CONFLICT (source_type, source_id) DO UPDATE SET
  caller_pool_id = EXCLUDED.caller_pool_id,
  egress_group_id = EXCLUDED.egress_group_id;

EOSQL

echo ""
echo "============================================================"
echo " ✅ 中继路由关联测试数据全量写入成功！"
echo "============================================================"
