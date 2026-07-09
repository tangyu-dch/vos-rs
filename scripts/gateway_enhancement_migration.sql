-- 网关类型、注册支持和前缀规则增强
-- gateway_type: peer(对接网关), gateway(落地网关), extension(分机)
-- 分机由落地网关自动创建，删除落地网关时级联删除

-- 添加网关类型字段
ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS gateway_type VARCHAR(20) DEFAULT 'peer';

-- 添加统一前缀规则字段
ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS prefix_rules TEXT DEFAULT '';

-- 添加注册支持开关
ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS supports_registration BOOLEAN DEFAULT false;

-- 分机关联字段：记录是哪个落地网关创建的分机
ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS parent_gateway_id TEXT DEFAULT '';

-- 清理旧字段（如果存在）
ALTER TABLE sip_gateways DROP COLUMN IF EXISTS prefix_add;
ALTER TABLE sip_gateways DROP COLUMN IF EXISTS prefix_strip;
ALTER TABLE sip_gateways DROP COLUMN IF EXISTS prefix_replace_from;
ALTER TABLE sip_gateways DROP COLUMN IF EXISTS prefix_replace_to;

-- 更新现有网关为 peer 类型
UPDATE sip_gateways SET gateway_type = 'peer' WHERE gateway_type IS NULL;

-- 添加注释
COMMENT ON COLUMN sip_gateways.gateway_type IS '网关类型: peer(对接), gateway(落地), extension(分机)';
COMMENT ON COLUMN sip_gateways.prefix_rules IS '前缀规则，逗号分隔: abc:def(替换) :def(添加) abc:(剥离)';
COMMENT ON COLUMN sip_gateways.supports_registration IS '是否支持第三方注册接入';
COMMENT ON COLUMN sip_gateways.parent_gateway_id IS '分机所属的落地网关ID（分机由落地网关创建时自动填充）';

-- 创建索引
CREATE INDEX IF NOT EXISTS idx_sip_gateways_type ON sip_gateways(gateway_type);
CREATE INDEX IF NOT EXISTS idx_sip_gateways_parent ON sip_gateways(parent_gateway_id);
