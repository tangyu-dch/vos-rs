//! Progressive schema for ingress authentication, caller identity and termination.

pub(crate) const MIGRATE_TERMINATION_DOMAIN_SQL: &[&str] = &[
    "ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS role TEXT NOT NULL DEFAULT 'egress'",
    "ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS access_auth_mode TEXT NOT NULL DEFAULT 'none'",
    "ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS access_username TEXT NOT NULL DEFAULT ''",
    "ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS access_realm TEXT NOT NULL DEFAULT ''",
    "ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS access_password_hash TEXT NOT NULL DEFAULT ''",
    r#"DO $$ BEGIN
        IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname='chk_gateway_termination_role') THEN
            ALTER TABLE sip_gateways ADD CONSTRAINT chk_gateway_termination_role
                CHECK (role IN ('access', 'egress')) NOT VALID;
        END IF;
        IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname='chk_gateway_access_auth_mode') THEN
            ALTER TABLE sip_gateways ADD CONSTRAINT chk_gateway_access_auth_mode CHECK (
                access_auth_mode IN ('none', 'ip_allowlist', 'digest_register', 'ip_and_digest')
                AND (role <> 'access' OR access_auth_mode <> 'none')
            ) NOT VALID;
        END IF;
        IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname='chk_gateway_digest_credentials') THEN
            ALTER TABLE sip_gateways ADD CONSTRAINT chk_gateway_digest_credentials CHECK (
                access_auth_mode NOT IN ('digest_register', 'ip_and_digest') OR
                (BTRIM(access_username) <> '' AND BTRIM(access_realm) <> '' AND BTRIM(access_password_hash) <> '')
            ) NOT VALID;
        END IF;
    END $$"#,
    r#"CREATE UNIQUE INDEX IF NOT EXISTS idx_access_gateway_username_unique
        ON sip_gateways (access_username)
        WHERE role='access' AND enabled
          AND access_auth_mode IN ('digest_register', 'ip_and_digest')
          AND BTRIM(access_username) <> ''"#,
    "ALTER TABLE number_inventory ADD COLUMN IF NOT EXISTS owner_egress_trunk_id TEXT REFERENCES sip_gateways(id) ON DELETE RESTRICT",
    r#"CREATE TABLE IF NOT EXISTS trunk_ip_rules (
        id BIGSERIAL PRIMARY KEY,
        trunk_id TEXT NOT NULL REFERENCES sip_gateways(id) ON DELETE CASCADE,
        cidr CIDR NOT NULL,
        source_port INTEGER,
        transport TEXT NOT NULL DEFAULT 'udp',
        description TEXT NOT NULL DEFAULT '',
        enabled BOOLEAN NOT NULL DEFAULT TRUE,
        created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
        UNIQUE (trunk_id, cidr, source_port, transport),
        CHECK (source_port IS NULL OR source_port BETWEEN 1 AND 65535),
        CHECK (transport = 'udp')
    )"#,
    r#"CREATE TABLE IF NOT EXISTS egress_endpoints (
        id BIGSERIAL PRIMARY KEY,
        trunk_id TEXT NOT NULL REFERENCES sip_gateways(id) ON DELETE CASCADE,
        host TEXT NOT NULL,
        port INTEGER NOT NULL DEFAULT 5060,
        transport TEXT NOT NULL DEFAULT 'udp',
        priority INTEGER NOT NULL DEFAULT 100,
        enabled BOOLEAN NOT NULL DEFAULT TRUE,
        created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
        UNIQUE (trunk_id, host, port, transport),
        CHECK (port BETWEEN 1 AND 65535),
        CHECK (priority BETWEEN 0 AND 65535),
        CHECK (transport = 'udp')
    )"#,
    r#"CREATE TABLE IF NOT EXISTS number_allocations (
        id BIGSERIAL PRIMARY KEY,
        number TEXT NOT NULL REFERENCES number_inventory(number) ON DELETE CASCADE,
        source_type TEXT NOT NULL,
        source_id TEXT NOT NULL,
        enabled BOOLEAN NOT NULL DEFAULT TRUE,
        created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
        UNIQUE (number, source_type, source_id),
        CHECK (source_type IN ('trunk', 'extension', 'extension_group'))
    )"#,
    r#"CREATE TABLE IF NOT EXISTS caller_pools (
        id TEXT PRIMARY KEY,
        owner_source_type TEXT NOT NULL,
        owner_source_id TEXT NOT NULL,
        virtual_alias TEXT NOT NULL,
        strategy TEXT NOT NULL DEFAULT 'random',
        fallback_mode TEXT NOT NULL DEFAULT 'reject',
        enabled BOOLEAN NOT NULL DEFAULT TRUE,
        created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
        updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
        UNIQUE (owner_source_type, owner_source_id, virtual_alias),
        CHECK (owner_source_type IN ('trunk', 'extension', 'extension_group')),
        CHECK (strategy IN ('random', 'round_robin', 'weighted_random', 'stable_hash', 'weighted', 'hash')),
        CHECK (fallback_mode IN ('reject', 'fallback_number', 'fallback_pool', 'fixed', 'pool'))
    )"#,
    r#"CREATE TABLE IF NOT EXISTS caller_pool_members (
        id BIGSERIAL PRIMARY KEY,
        pool_id TEXT NOT NULL REFERENCES caller_pools(id) ON DELETE CASCADE,
        number TEXT NOT NULL REFERENCES number_inventory(number) ON DELETE RESTRICT,
        priority INTEGER NOT NULL DEFAULT 100,
        weight INTEGER NOT NULL DEFAULT 100,
        max_concurrent INTEGER NOT NULL DEFAULT 0,
        enabled BOOLEAN NOT NULL DEFAULT TRUE,
        created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
        UNIQUE (pool_id, number),
        CHECK (priority BETWEEN 0 AND 65535),
        CHECK (weight BETWEEN 1 AND 10000),
        CHECK (max_concurrent >= 0)
    )"#,
    r#"CREATE TABLE IF NOT EXISTS egress_groups (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        description TEXT NOT NULL DEFAULT '',
        enabled BOOLEAN NOT NULL DEFAULT TRUE,
        created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
        updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
    )"#,
    r#"CREATE TABLE IF NOT EXISTS egress_group_members (
        id BIGSERIAL PRIMARY KEY,
        group_id TEXT NOT NULL REFERENCES egress_groups(id) ON DELETE CASCADE,
        egress_trunk_id TEXT NOT NULL REFERENCES sip_gateways(id) ON DELETE RESTRICT,
        destination_prefix TEXT NOT NULL DEFAULT '',
        priority INTEGER NOT NULL DEFAULT 100,
        weight INTEGER NOT NULL DEFAULT 100,
        time_start TEXT,
        time_end TEXT,
        enabled BOOLEAN NOT NULL DEFAULT TRUE,
        created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
        UNIQUE (group_id, egress_trunk_id, destination_prefix),
        CHECK (priority BETWEEN 0 AND 65535),
        CHECK (weight BETWEEN 1 AND 10000)
    )"#,
    r#"CREATE TABLE IF NOT EXISTS source_outbound_policies (
        source_type TEXT NOT NULL,
        source_id TEXT NOT NULL,
        caller_mode TEXT NOT NULL,
        fixed_number TEXT REFERENCES number_inventory(number) ON DELETE RESTRICT,
        caller_pool_id TEXT REFERENCES caller_pools(id) ON DELETE RESTRICT,
        egress_mode TEXT NOT NULL,
        direct_egress_trunk_id TEXT REFERENCES sip_gateways(id) ON DELETE RESTRICT,
        egress_group_id TEXT REFERENCES egress_groups(id) ON DELETE RESTRICT,
        fallback_mode TEXT NOT NULL DEFAULT 'reject',
        enabled BOOLEAN NOT NULL DEFAULT TRUE,
        updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
        PRIMARY KEY (source_type, source_id),
        CHECK (source_type IN ('trunk', 'extension', 'extension_group')),
        CHECK (caller_mode IN ('strict_passthrough', 'fixed_number', 'virtual_pool')),
        CHECK (egress_mode IN ('direct', 'group')),
        CHECK (fallback_mode IN ('reject', 'fallback_number', 'fallback_pool', 'fixed', 'pool')),
        CHECK ((caller_mode = 'strict_passthrough' AND fixed_number IS NULL AND caller_pool_id IS NULL)
            OR (caller_mode = 'fixed_number' AND fixed_number IS NOT NULL AND caller_pool_id IS NULL)
            OR (caller_mode = 'virtual_pool' AND fixed_number IS NULL AND caller_pool_id IS NOT NULL)),
        CHECK ((egress_mode = 'direct' AND direct_egress_trunk_id IS NOT NULL AND egress_group_id IS NULL)
            OR (egress_mode = 'group' AND direct_egress_trunk_id IS NULL AND egress_group_id IS NOT NULL))
    )"#,
    r#"CREATE TABLE IF NOT EXISTS did_destinations (
        number TEXT PRIMARY KEY REFERENCES number_inventory(number) ON DELETE CASCADE,
        tenant_id TEXT,
        target_type TEXT NOT NULL,
        target_id TEXT NOT NULL,
        enabled BOOLEAN NOT NULL DEFAULT TRUE,
        updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
        CHECK (target_type IN ('extension', 'extension_group', 'ivr', 'reject'))
    )"#,
    "CREATE INDEX IF NOT EXISTS idx_trunk_ip_rules_trunk ON trunk_ip_rules (trunk_id)",
    "ALTER TABLE trunk_ip_rules ADD COLUMN IF NOT EXISTS description TEXT NOT NULL DEFAULT ''",
    "CREATE INDEX IF NOT EXISTS idx_egress_endpoints_trunk ON egress_endpoints (trunk_id)",
    "CREATE INDEX IF NOT EXISTS idx_number_owner ON number_inventory (owner_egress_trunk_id)",
    "CREATE INDEX IF NOT EXISTS idx_number_allocations_source ON number_allocations (source_type, source_id)",
    r#"INSERT INTO number_allocations(number,source_type,source_id,enabled)
        SELECT number,'extension',username,TRUE FROM number_inventory
        WHERE username IS NOT NULL AND BTRIM(username) <> ''
          AND NOT EXISTS (SELECT 1 FROM number_allocations a WHERE a.number=number_inventory.number AND a.enabled)
        ON CONFLICT (number,source_type,source_id) DO NOTHING"#,
    "CREATE UNIQUE INDEX IF NOT EXISTS idx_number_allocations_one_active ON number_allocations (number) WHERE enabled",
    "CREATE INDEX IF NOT EXISTS idx_caller_pool_members_pool ON caller_pool_members (pool_id)",
    "CREATE INDEX IF NOT EXISTS idx_egress_group_members_group ON egress_group_members (group_id)",
    "ALTER TABLE egress_groups ADD COLUMN IF NOT EXISTS description TEXT NOT NULL DEFAULT ''",
    r#"DO $$ BEGIN
        IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname='chk_source_policy_caller_fields') THEN
            ALTER TABLE source_outbound_policies ADD CONSTRAINT chk_source_policy_caller_fields CHECK (
                (caller_mode='strict_passthrough' AND fixed_number IS NULL AND caller_pool_id IS NULL) OR
                (caller_mode='fixed_number' AND fixed_number IS NOT NULL AND caller_pool_id IS NULL) OR
                (caller_mode='virtual_pool' AND fixed_number IS NULL AND caller_pool_id IS NOT NULL)
            ) NOT VALID;
        END IF;
        IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname='chk_source_policy_egress_fields') THEN
            ALTER TABLE source_outbound_policies ADD CONSTRAINT chk_source_policy_egress_fields CHECK (
                (egress_mode='direct' AND direct_egress_trunk_id IS NOT NULL AND egress_group_id IS NULL) OR
                (egress_mode='group' AND direct_egress_trunk_id IS NULL AND egress_group_id IS NOT NULL)
            ) NOT VALID;
        END IF;
    END $$"#,
    r#"CREATE TABLE IF NOT EXISTS extension_groups (
        id VARCHAR(50) PRIMARY KEY,
        name VARCHAR(100) NOT NULL,
        description TEXT DEFAULT '',
        created_at TIMESTAMPTZ NOT NULL DEFAULT now()
    )"#,
    r#"CREATE TABLE IF NOT EXISTS extension_group_members (
        group_id VARCHAR(50) REFERENCES extension_groups(id) ON DELETE CASCADE,
        username VARCHAR(50) REFERENCES sip_users(username) ON DELETE CASCADE,
        PRIMARY KEY (group_id, username)
    )"#,
    r#"CREATE TABLE IF NOT EXISTS ivr_menus (
        id VARCHAR(50) PRIMARY KEY,
        name VARCHAR(100) NOT NULL,
        welcome_prompt VARCHAR(255) NOT NULL DEFAULT 'welcome.wav',
        timeout_secs INTEGER NOT NULL DEFAULT 10,
        description TEXT DEFAULT '',
        did VARCHAR(50) DEFAULT '',
        enabled BOOLEAN NOT NULL DEFAULT TRUE,
        nodes JSONB NOT NULL DEFAULT '[]'::jsonb,
        edges JSONB NOT NULL DEFAULT '[]'::jsonb,
        created_at TIMESTAMPTZ NOT NULL DEFAULT now()
    )"#,
    r#"CREATE TABLE IF NOT EXISTS ivr_actions (
        ivr_id VARCHAR(50) REFERENCES ivr_menus(id) ON DELETE CASCADE,
        dtmf_key VARCHAR(5) NOT NULL,
        action_type VARCHAR(20) NOT NULL,
        action_target VARCHAR(100) NOT NULL,
        waiting_prompt VARCHAR(255),
        webhook_method VARCHAR(10),
        PRIMARY KEY (ivr_id, dtmf_key)
    )"#,
    // 旧库升级: 为已存在的 ivr_menus 补充新字段
    r#"ALTER TABLE ivr_menus ADD COLUMN IF NOT EXISTS description TEXT DEFAULT ''"#,
    r#"ALTER TABLE ivr_menus ADD COLUMN IF NOT EXISTS did VARCHAR(50) DEFAULT ''"#,
    r#"ALTER TABLE ivr_menus ADD COLUMN IF NOT EXISTS enabled BOOLEAN NOT NULL DEFAULT TRUE"#,
    r#"ALTER TABLE ivr_menus ADD COLUMN IF NOT EXISTS nodes JSONB NOT NULL DEFAULT '[]'::jsonb"#,
    r#"ALTER TABLE ivr_menus ADD COLUMN IF NOT EXISTS edges JSONB NOT NULL DEFAULT '[]'::jsonb"#,
    r#"ALTER TABLE ivr_actions ADD COLUMN IF NOT EXISTS waiting_prompt VARCHAR(255)"#,
    r#"ALTER TABLE ivr_actions ADD COLUMN IF NOT EXISTS webhook_method VARCHAR(10)"#,
    r#"CREATE TABLE IF NOT EXISTS call_queues (
        id VARCHAR(50) PRIMARY KEY,
        name VARCHAR(100) NOT NULL,
        strategy VARCHAR(30) NOT NULL DEFAULT 'longest_idle',
        moh_file VARCHAR(255) NOT NULL DEFAULT 'moh.wav',
        max_wait_secs INTEGER NOT NULL DEFAULT 300,
        created_at TIMESTAMPTZ NOT NULL DEFAULT now()
    )"#,
    r#"CREATE TABLE IF NOT EXISTS call_agents (
        agent_id VARCHAR(50) PRIMARY KEY,
        name VARCHAR(100) NOT NULL,
        extension VARCHAR(50) NOT NULL,
        status VARCHAR(20) NOT NULL DEFAULT 'idle',
        created_at TIMESTAMPTZ NOT NULL DEFAULT now()
    )"#,
    r#"CREATE TABLE IF NOT EXISTS queue_agents (
        queue_id VARCHAR(50) REFERENCES call_queues(id) ON DELETE CASCADE,
        agent_id VARCHAR(50) REFERENCES call_agents(agent_id) ON DELETE CASCADE,
        penalty INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (queue_id, agent_id)
    )"#,
];

#[cfg(test)]
mod tests {
    use super::MIGRATE_TERMINATION_DOMAIN_SQL;

    #[test]
    fn migration_is_progressive_and_idempotent() {
        let sql = MIGRATE_TERMINATION_DOMAIN_SQL
            .join("\n")
            .to_ascii_uppercase();
        assert!(!sql.contains("DROP TABLE"));
        assert!(!sql.contains("DROP COLUMN"));
        assert!(sql.contains("IF NOT EXISTS"));
    }

    #[test]
    fn policy_and_number_constraints_are_present() {
        let sql = MIGRATE_TERMINATION_DOMAIN_SQL.join("\n");
        assert!(sql.contains("idx_number_allocations_one_active"));
        assert!(sql.contains("chk_source_policy_caller_fields"));
        assert!(sql.contains("chk_source_policy_egress_fields"));
        assert!(sql.contains("owner_egress_trunk_id"));
        assert!(sql.contains("idx_access_gateway_username_unique"));
        assert!(sql.contains("chk_gateway_access_auth_mode"));
        assert!(sql.contains("chk_gateway_digest_credentials"));
    }
}
