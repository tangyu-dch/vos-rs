-- billing_accounts 改用自增 ID 作为主键
-- 保留 username 作为唯一约束

-- 1. 添加自增 ID 列（带默认值）
ALTER TABLE billing_accounts ADD COLUMN IF NOT EXISTS id BIGSERIAL;

-- 2. 为现有行填充 ID（如果有的话）
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM billing_accounts LIMIT 1) THEN
        -- 创建序列并设置当前值
        PERFORM setval(pg_get_serial_sequence('billing_accounts', 'id'), 
                       (SELECT COALESCE(MAX(id), 0) + 1 FROM billing_accounts));
    END IF;
END $$;

-- 3. 设置 ID 为 NOT NULL
ALTER TABLE billing_accounts ALTER COLUMN id SET NOT NULL;

-- 4. 删除旧的主键约束（username）
ALTER TABLE billing_accounts DROP CONSTRAINT IF EXISTS billing_accounts_pkey;

-- 5. 添加新的主键约束（id）
ALTER TABLE billing_accounts ADD PRIMARY KEY (id);

-- 6. 添加 username 唯一约束
ALTER TABLE billing_accounts ADD CONSTRAINT uq_billing_accounts_username UNIQUE (username);

-- 7. sip_gateways 添加 account_id 关联字段
ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS account_id BIGINT DEFAULT NULL;

-- 添加外键约束（如果不存在）
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM information_schema.table_constraints 
        WHERE constraint_name = 'fk_gateway_account'
    ) THEN
        ALTER TABLE sip_gateways 
        ADD CONSTRAINT fk_gateway_account 
        FOREIGN KEY (account_id) REFERENCES billing_accounts(id) ON DELETE SET NULL;
    END IF;
END $$;

COMMENT ON COLUMN sip_gateways.account_id IS '关联的计费账户ID（对接网关用于计费）';
COMMENT ON COLUMN billing_accounts.id IS '自增主键';
COMMENT ON COLUMN billing_accounts.username IS '账户用户名（唯一约束）';

CREATE INDEX IF NOT EXISTS idx_sip_gateways_account ON sip_gateways(account_id);
