#!/usr/bin/env bash
# ====================================================================
# Vos-rs 一键清理历史运营数据 Shell 脚本
# 仅保留：中继 (Trunks)、号码 (Numbers)、账号 (Billing Accounts)、分机 (Extensions)
# ====================================================================

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
SQL_FILE="$SCRIPT_DIR/clean_history_data.sql"

# 读取环境变量或设置默认值
DATABASE_URL="${VOS_RS_DATABASE_URL:-postgres://postgres:postgres@localhost:5432/vos_rs}"

echo "===================================================="
echo " [Vos-rs] 数据库历史运营数据一键清理工具"
echo "===================================================="
echo " 校验范围："
echo "   - 保留中继 (sip_gateways, egress_groups, trunk_ip_rules)"
echo "   - 保留号码 (number_inventory, did_destinations, caller_pools)"
echo "   - 保留账号 (billing_accounts, billing_rates, system_configs)"
echo "   - 保留分机 (sip_extensions, sip_users, call_queues, ivr_menus)"
echo "   - 清空 CDR、DTMF、SIP 抓包、注册状态、流水明细、风控事件"
echo "===================================================="

# 尝试通过 psql 执行
if command -v psql &> /dev/null; then
    echo "[+] 使用本地 psql 工具连接数据库执行清理..."
    psql "$DATABASE_URL" -f "$SQL_FILE"
# 尝试通过 docker compose 执行
elif docker compose version &> /dev/null && docker compose ps postgres &> /dev/null; then
    echo "[+] 使用 Docker Container (postgres) 执行清理..."
    docker compose exec -T postgres psql -U postgres -d vos_rs < "$SQL_FILE"
else
    echo "[-] 未检测到 psql 工具或正在运行的 Docker Postgres 容器。"
    echo "    请直接连接 PostgreSQL 并执行以下 SQL 文件："
    echo "    $SQL_FILE"
    exit 1
fi

echo ""
echo "===================================================="
echo " [√] 历史运营数据已成功清理完毕！核心配置已完整保留。"
echo "===================================================="
