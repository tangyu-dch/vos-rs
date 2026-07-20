-- ====================================================================
-- Vos-rs 电信软交换平台：历史与动态运行数据彻底清理脚本
-- 
-- 规则说明：
--   1. 【保留】核心业务配置：中继 (Gateways/Trunks)、号码 (Numbers/DIDs)、账号 (Accounts/Rates)、分机 (Extensions/Users/Queues/IVR/Agents)。
--   2. 【清空】历史与动态运营数据：CDR 通话记录、DTMF 按键日志、SIP 信令流日志、注册状态、探活记录、账务变动明细、安全风控事件日志、API 审计日志。
-- ====================================================================

BEGIN;

-- 1. 彻底清空通话 CDR 详单与信令流历史
TRUNCATE TABLE call_cdrs CASCADE;
TRUNCATE TABLE dtmf_events CASCADE;
TRUNCATE TABLE sip_flows CASCADE;

-- 2. 彻底清空注册缓存与探活历史（节点运行后会自动重新建立）
TRUNCATE TABLE sip_registrations CASCADE;
TRUNCATE TABLE gateway_health_status CASCADE;

-- 3. 彻底清空账务充值与扣款变动流水（保留 billing_accounts 账户本身与余额）
TRUNCATE TABLE billing_ledger CASCADE;

-- 4. 彻底清空安全风控历史拦截事件与 API 审计日志
TRUNCATE TABLE anti_fraud_events CASCADE;
TRUNCATE TABLE api_audit_logs CASCADE;

COMMIT;

-- 5. 执行空间回收与索引整理
VACUUM ANALYZE call_cdrs;
VACUUM ANALYZE dtmf_events;
VACUUM ANALYZE sip_flows;
VACUUM ANALYZE sip_registrations;
VACUUM ANALYZE gateway_health_status;
VACUUM ANALYZE billing_ledger;
VACUUM ANALYZE anti_fraud_events;
VACUUM ANALYZE api_audit_logs;
