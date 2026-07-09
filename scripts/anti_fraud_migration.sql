-- 防盗打规则表
CREATE TABLE IF NOT EXISTS anti_fraud_rules (
    id BIGSERIAL PRIMARY KEY,
    rule_type VARCHAR(50) NOT NULL,  -- 'blocked_prefix', 'allowed_prefix', 'blocked_ip', 'allowed_ip'
    value VARCHAR(255) NOT NULL,     -- 号码前缀或IP地址/CIDR
    description TEXT,
    enabled BOOLEAN DEFAULT true,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW(),
    UNIQUE(rule_type, value)
);

-- 防盗打全局配置表
CREATE TABLE IF NOT EXISTS anti_fraud_config (
    id BIGSERIAL PRIMARY KEY,
    config_key VARCHAR(100) NOT NULL UNIQUE,
    config_value TEXT NOT NULL,
    description TEXT,
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

-- 防盗打事件日志表
CREATE TABLE IF NOT EXISTS anti_fraud_events (
    id BIGSERIAL PRIMARY KEY,
    event_type VARCHAR(50) NOT NULL,  -- 'blocked_number', 'blocked_ip', 'concurrent_exceeded', etc.
    source_ip INET,
    account VARCHAR(100),
    destination VARCHAR(200),
    detail TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

-- 插入默认配置
INSERT INTO anti_fraud_config (config_key, config_value, description) VALUES
('enabled', 'true', '启用防盗打'),
('max_concurrent_per_account', '50', '每账户最大并发呼叫数'),
('max_concurrent_per_ip', '20', '每IP最大并发呼叫数'),
('max_cps_per_account', '10', '每账户每秒最大呼叫数'),
('min_call_duration', '3', '最短通话时长（秒）'),
('max_call_duration', '3600', '最长通话时长（秒）'),
('short_call_threshold', '5', '短通话检测阈值'),
('short_call_window', '60', '短通话检测窗口（秒）'),
('block_international', 'true', '拦截国际呼叫'),
('block_premium', 'true', '拦截高额号码'),
('allow_zero_balance', 'false', '允许零余额呼叫')
ON CONFLICT (config_key) DO NOTHING;

-- 插入默认黑名单号码前缀
INSERT INTO anti_fraud_rules (rule_type, value, description) VALUES
('blocked_prefix', '116', '高额号码'),
('blocked_prefix', '168', '高额号码'),
('blocked_prefix', '160', '高额号码'),
('blocked_prefix', '161', '高额号码'),
('blocked_prefix', '162', '高额号码'),
('blocked_prefix', '167', '高额号码')
ON CONFLICT (rule_type, value) DO NOTHING;

-- 索引
CREATE INDEX IF NOT EXISTS idx_anti_fraud_rules_type ON anti_fraud_rules(rule_type);
CREATE INDEX IF NOT EXISTS idx_anti_fraud_rules_enabled ON anti_fraud_rules(enabled);
CREATE INDEX IF NOT EXISTS idx_anti_fraud_events_type ON anti_fraud_events(event_type);
CREATE INDEX IF NOT EXISTS idx_anti_fraud_events_created ON anti_fraud_events(created_at);
CREATE INDEX IF NOT EXISTS idx_anti_fraud_events_source_ip ON anti_fraud_events(source_ip);
