#!/usr/bin/env python3
"""截取 vos-rs Web 控制台核心页面到 docs/assets/

依赖: /opt/miniconda3/bin/python (含 playwright)
用法: /opt/miniconda3/bin/python scripts/screenshot.py
"""
import sys
from pathlib import Path
from playwright.sync_api import sync_playwright

BASE = "http://localhost:3001"
OUT = Path(__file__).parent.parent / "docs" / "assets"
OUT.mkdir(parents=True, exist_ok=True)

# (路径, 文件名, 等待选择器)
PAGES = [
    ("/login", "login.png", 'input[type=password]'),
    ("/overview", "dashboard.png", "body"),
    ("/calls/active", "active-calls.png", "body"),
    ("/extensions", "extensions.png", "body"),
    ("/trunks/access", "trunks.png", "body"),
    ("/routing", "routing.png", "body"),
    ("/billing/accounts", "billing-accounts.png", "body"),
    ("/settings", "settings.png", "body"),
    ("/security", "security.png", "body"),
    ("/infrastructure", "infrastructure.png", "body"),
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


def shot(context, path, filename, wait_sel):
    """截单个页面"""
    page = context.new_page()
    try:
        page.goto(f"{BASE}{path}", wait_until="networkidle", timeout=15000)
        page.wait_for_timeout(1500)
        out_path = OUT / filename
        page.screenshot(path=str(out_path), full_page=True)
        print(f"[OK] {path} -> {out_path.name} ({out_path.stat().st_size} bytes)")
    except Exception as e:
        print(f"[FAIL] {path}: {e}", file=sys.stderr)
    finally:
        page.close()


def main():
    with sync_playwright() as p:
        browser = p.chromium.launch(headless=True)

        # 1. 未登录 context 截登录页
        lctx = browser.new_context(
            viewport={"width": 1440, "height": 900},
            device_scale_factor=2,
        )
        lctx.add_init_script(
            "try { localStorage.setItem('vos-theme','dark'); } catch(e) {}"
        )
        shot(lctx, "/login", "login.png", 'input[type=password]')
        lctx.close()

        # 2. 已登录 context 截其他页面
        ctx = browser.new_context(
            viewport={"width": 1440, "height": 900},
            device_scale_factor=2,
        )
        ctx.add_init_script(
            "try { localStorage.setItem('vos-theme','dark'); } catch(e) {}"
        )
        login(ctx)
        for path, filename, wait_sel in PAGES[1:]:  # 跳过 login
            shot(ctx, path, filename, wait_sel)
        ctx.close()

        # 3. 窄屏 settings (800x900, 验证自适应)
        mctx = browser.new_context(
            viewport={"width": 800, "height": 900},
            device_scale_factor=2,
        )
        mctx.add_init_script(
            "try { localStorage.setItem('vos-theme','dark'); } catch(e) {}"
        )
        login(mctx)
        shot(mctx, "/settings", "settings-narrow.png", "body")
        mctx.close()

        browser.close()


if __name__ == "__main__":
    main()
