#!/usr/bin/env python3
"""截取 vos-rs Web 控制台核心页面到 docs/assets/（暗黑主题）

依赖: /opt/miniconda3/bin/python (含 playwright)
用法: /opt/miniconda3/bin/python scripts/screenshot.py
"""
import sys
from pathlib import Path
from playwright.sync_api import sync_playwright

BASE = "http://localhost:3001"
OUT = Path(__file__).parent.parent / "docs" / "assets"
OUT.mkdir(parents=True, exist_ok=True)

# (路径, 文件名)  登录页单独处理
PAGES = [
    ("/overview", "dashboard.png"),
    ("/calls/active", "active-calls.png"),
    ("/calls", "calls.png"),
    ("/copilot", "copilot.png"),
    ("/extensions", "extensions.png"),
    ("/numbers", "numbers.png"),
    ("/caller-pools", "caller-pools.png"),
    ("/trunks/access", "trunks.png"),
    ("/egress-groups", "egress-groups.png"),
    ("/ivr", "ivr.png"),
    ("/queues", "queues.png"),
    ("/agents", "agents.png"),
    ("/routing", "routing.png"),
    ("/billing/accounts", "billing-accounts.png"),
    ("/billing/rates", "billing-rates.png"),
    ("/billing/transactions", "billing-transactions.png"),
    ("/settings", "settings.png"),
    ("/settings/llm", "llm-configs.png"),
    ("/security", "security.png"),
    ("/infrastructure", "infrastructure.png"),
]


def login(context):
    """登录后台"""
    page = context.new_page()
    page.goto(f"{BASE}/login", wait_until="networkidle")
    page.fill('input[placeholder*="用户名"], input[type="text"]', "admin")
    page.fill('input[type="password"]', "admin")
    page.click('button[type="submit"]')
    page.wait_for_url("**/overview", timeout=10000)
    print(f"[LOGIN] 登录成功 -> {page.url}")
    page.close()


def shot(context, path, filename):
    """截单个页面（暗黑主题，等 React 渲染稳定后截全页）"""
    page = context.new_page()
    try:
        page.goto(f"{BASE}{path}", wait_until="networkidle", timeout=15000)
        # 等待 React 渲染 + HeroUI 暗黑主题 class 生效
        page.wait_for_timeout(2000)
        out_path = OUT / filename
        page.screenshot(path=str(out_path), full_page=True)
        print(f"[OK] {path} -> {out_path.name} ({out_path.stat().st_size} bytes)")
    except Exception as e:
        print(f"[FAIL] {path}: {e}", file=sys.stderr)
    finally:
        page.close()


def make_context(browser, width=1440, height=900, scale=2):
    """创建暗黑主题 context（viewport 1440x900 @2x retina）"""
    ctx = browser.new_context(
        viewport={"width": width, "height": height},
        device_scale_factor=scale,
    )
    # 在每个页面加载前注入暗黑主题
    ctx.add_init_script(
        "try { localStorage.setItem('vos-theme','dark'); } catch(e) {}"
    )
    return ctx


def main():
    with sync_playwright() as p:
        browser = p.chromium.launch(headless=True)

        # 1. 未登录 context 截登录页
        lctx = make_context(browser)
        shot(lctx, "/login", "login.png")
        lctx.close()

        # 2. 已登录 context 截全部业务页面（桌面宽屏 1440x900 @2x）
        ctx = make_context(browser)
        login(ctx)
        for path, filename in PAGES:
            shot(ctx, path, filename)
        ctx.close()

        # 3. 窄屏 settings（800x900，验证响应式 2 列布局）
        mctx = make_context(browser, width=800, height=900)
        login(mctx)
        shot(mctx, "/settings", "settings-narrow.png")
        mctx.close()

        browser.close()
        print(f"\n[DONE] 截图已保存到 {OUT}/")


if __name__ == "__main__":
    main()
