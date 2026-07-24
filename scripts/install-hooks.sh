#!/usr/bin/env bash
# ==============================================================================
# vos-rs Git Pre-commit Hook 一键安装脚本
# 用途：为开发者设置本地 git pre-commit hook，确保提交前自动运行 clippy、fmt 和 test
# ==============================================================================

set -euo pipefail

HOOK_PATH=".git/hooks/pre-commit"

echo "⚓️ 正在安装 vos-rs Git Pre-commit Hook..."

mkdir -p .git/hooks

cat > "${HOOK_PATH}" <<'EOF'
#!/usr/bin/env bash
set -e

echo "🔍 [Pre-commit] 检查 Rust 代码格式 (cargo fmt)..."
cargo fmt -- --check

echo "🧹 [Pre-commit] 运行静态分析 (cargo clippy)..."
cargo clippy --workspace --all-targets -- -D warnings

echo "🧪 [Pre-commit] 运行单元测试集 (cargo test)..."
cargo test --workspace

echo "✅ [Pre-commit] 代码质量检查通过，允许 Commit！"
EOF

chmod +x "${HOOK_PATH}"

echo "✅ Pre-commit hook 安装成功！下次进行 git commit 时将自动触发检查。"
