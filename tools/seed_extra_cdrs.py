#!/usr/bin/env python3
"""向 vos_rs 库补充 failed/canceled CDR，让仪表板状态分布环更丰富。

注意：这些是手造数据，非真实呼叫，仅用于演示。所有 call_id 以 'seed-' 前缀标识，
可用 `DELETE FROM call_cdrs WHERE call_id LIKE 'seed-%'` 单独清理，不影响真实 CDR。

用法:
    python3 tools/seed_extra_cdrs.py                 # 插入 8 条
    python3 tools/seed_extra_cdrs.py --count 12      # 插入 12 条
    python3 tools/seed_extra_cdrs.py --clean          # 先清旧 seed 再插入

环境变量:
    VOS_RS_FULL_FLOW_DATABASE_URL / DATABASE_URL  默认 postgres://vos_rs:vos_rs@127.0.0.1:5432/vos_rs
"""
import argparse
import os
import random
import subprocess
import sys
import time
from datetime import datetime, timedelta, timezone

DEFAULT_DB = "postgres://vos_rs:vos_rs@127.0.0.1:5432/vos_rs"

# 失败呼叫样例：(SIP 状态码, 原因, 被叫)
FAILED_CASES = [
    (480, "Temporarily Unavailable", "13900139000"),
    (486, "Busy Here", "13700137000"),
    (503, "Service Unavailable", "13600136000"),
    (408, "Request Timeout", "13500135000"),
    (484, "Address Incomplete", "13400134000"),
]
CANCELED_CALLEES = ["13800138000", "13900139000", "13700137000", "13600136000"]
CALLERS = ["1001", "1003", "1004", "1005"]


def rand_time_today():
    """UTC 今日 0 点 ~ 当前 之间的随机时间（确保落在仪表板今日统计窗口内）。"""
    now = datetime.now(timezone.utc)
    midnight = now.replace(hour=0, minute=0, second=0, microsecond=0)
    span = (now - midnight).total_seconds()
    # 若当前恰好在 0 点附近，span 很小，退化为今日 0 点。
    return midnight + timedelta(seconds=random.uniform(0, max(span, 1)))


def iso(dt):
    return dt.strftime("%Y-%m-%dT%H:%M:%S.") + f"{dt.microsecond // 1000:03d}Z"


def sql_str(s):
    """单引号字符串字面量（数据均为脚本生成、不含单引号，安全）。"""
    return f"'{s}'"


def build_rows(count):
    rows = []
    ts = int(time.time())
    for i in range(count):
        # ~55% 失败，~45% 取消
        if random.random() < 0.55:
            code, reason, callee = random.choice(FAILED_CASES)
            status = "failed"
            code_sql = str(code)
            reason_sql = sql_str(reason)
        else:
            code, reason = None, None
            callee = random.choice(CANCELED_CALLEES)
            status = "canceled"
            code_sql = "NULL"
            reason_sql = "NULL"

        caller_user = random.choice(CALLERS)
        caller = f'"{caller_user}" <sip:{caller_user}@127.0.0.1:{50000 + i}>;tag=seed-{ts}-{i}'
        call_id = f"seed-{status}-{ts}-{i}@vos-rs.local"
        started = rand_time_today()
        duration = random.randint(500, 8000)
        ended = started + timedelta(milliseconds=duration)

        # 未接通：answered_at=NULL，billable=0，mos/dtmf=NULL
        rows.append(
            f"({sql_str(call_id)}, {sql_str(caller)}, {sql_str(callee)}, "
            f"{sql_str(iso(started))}, NULL, {sql_str(iso(ended))}, "
            f"{duration}, 0, {sql_str(status)}, {code_sql}, {reason_sql}, NULL, NULL)"
        )
    return rows


def run_psql(db_url, sql, capture=False):
    return subprocess.run(
        ["psql", db_url, "-v", "ON_ERROR_STOP=1", "-c", sql],
        check=True,
        capture_output=capture,
        text=capture,
    )


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--count", type=int, default=8, help="插入条数（默认 8）")
    ap.add_argument("--clean", action="store_true", help="先删除 call_id LIKE 'seed-%%' 的旧数据再插入")
    args = ap.parse_args()

    db_url = (
        os.environ.get("VOS_RS_FULL_FLOW_DATABASE_URL")
        or os.environ.get("DATABASE_URL")
        or DEFAULT_DB
    )

    if args.clean:
        run_psql(db_url, "DELETE FROM call_cdrs WHERE call_id LIKE 'seed-%'")
        print("已清理旧的 seed CDR")

    rows = build_rows(args.count)
    sql = (
        "INSERT INTO call_cdrs (call_id, caller, callee, started_at, answered_at, ended_at, "
        "duration_ms, billable_duration_ms, status, failure_status_code, failure_reason, mos, dtmf_digits) VALUES\n"
        + ",\n".join(rows)
        + ";"
    )
    run_psql(db_url, sql)
    print(f"已插入 {args.count} 条 seed CDR（failed/canceled，call_id 前缀 'seed-'）")

    out = run_psql(
        db_url,
        "SELECT status, count(*) FROM call_cdrs GROUP BY status ORDER BY status;",
        capture=True,
    )
    print("\n当前 call_cdrs 状态分布:")
    print(out.stdout.strip())

    return 0


if __name__ == "__main__":
    sys.exit(main())
