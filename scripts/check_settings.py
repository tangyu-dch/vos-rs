#!/usr/bin/env python3
"""验证 settings 页面在不同 viewport 下的自适应布局"""
from pathlib import Path
from playwright.sync_api import sync_playwright

BASE = "http://localhost:3001"
OUT = Path(__file__).parent.parent / "docs" / "assets"
OUT.mkdir(parents=True, exist_ok=True)


def login(context):
    page = context.new_page()
    page.goto(f"{BASE}/login", wait_until="networkidle")
    page.fill('input[placeholder*="用户名"], input[type="text"]', "admin")
    page.fill('input[type="password"]', "admin")
    page.click('button[type="submit"]')
    page.wait_for_url("**/overview", timeout=10000)
    page.close()


def main():
    with sync_playwright() as p:
        browser = p.chromium.launch(headless=True)

        # 测试三个 viewport
        for width, label in [(1440, "desktop"), (800, "narrow"), (400, "mobile")]:
            ctx = browser.new_context(
                viewport={"width": width, "height": 900},
                device_scale_factor=2,
            )
            ctx.add_init_script(
                "try { localStorage.setItem('vos-theme','dark'); } catch(e) {}"
            )
            login(ctx)
            page = ctx.new_page()
            page.goto(f"{BASE}/settings", wait_until="networkidle", timeout=15000)
            page.wait_for_timeout(1500)
            out = OUT / f"settings-{label}.png"
            page.screenshot(path=str(out), full_page=True)
            print(f"[OK] {label} ({width}px) -> {out.name} ({out.stat().st_size} bytes)")
            # 获取主内容区实际宽度
            info = page.evaluate("""() => {
                const main = document.querySelector('main') || document.body;
                const grid = main.querySelector('[class*="grid-cols-1"]');
                return {
                    viewport: window.innerWidth,
                    mainWidth: main.getBoundingClientRect().width,
                    gridWidth: grid ? grid.getBoundingClientRect().width : null,
                    gridClass: grid ? grid.className : null,
                };
            }""")
            print(f"  -> {info}")
            page.close()
            ctx.close()

        browser.close()


if __name__ == "__main__":
    main()
