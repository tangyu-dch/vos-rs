#!/usr/bin/env bash
# VOS-RS Linux 部署脚本
# 用法: sudo ./deploy.sh [install|uninstall|status|logs]
set -euo pipefail

APP_NAME="vos-rs"
INSTALL_DIR="/opt/vos-rs"
CONFIG_DIR="/etc/vos-rs"
DATA_DIR="/opt/vos-rs/data"
LOG_DIR="/var/log/vos-rs"
SERVICE_USER="vos-rs"
SERVICE_GROUP="vos-rs"
SERVICES=("sip-router" "sip-edge" "media-edge" "api-server" "cdr-worker")

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

info()  { echo -e "${GREEN}[INFO]${NC} $*"; }
warn()  { echo -e "${YELLOW}[WARN]${NC} $*"; }
error() { echo -e "${RED}[ERROR]${NC} $*"; exit 1; }

check_root() {
    [[ $EUID -eq 0 ]] || error "请使用 root 权限运行: sudo $0 $*"
}

create_user() {
    if ! id "$SERVICE_USER" &>/dev/null; then
        info "创建用户 $SERVICE_USER"
        useradd -r -s /sbin/nologin -d "$INSTALL_DIR" "$SERVICE_USER"
    fi
}

install_binaries() {
    info "安装二进制文件到 $INSTALL_DIR"
    mkdir -p "$INSTALL_DIR/bin" "$DATA_DIR/recordings" "$LOG_DIR"

    # 从构建目录拷贝
    local profile="release"
    local target_dir="target/$profile"

    if [[ ! -f "$target_dir/sip-router" || ! -f "$target_dir/sip-edge" \
        || ! -f "$target_dir/media-edge" || ! -f "$target_dir/api-server" \
        || ! -f "$target_dir/cdr-worker" ]]; then
        warn "未找到 release 构建，正在编译..."
        cargo build --release -p sip-router -p sip-edge -p media-edge -p api-server -p cdr-worker
    fi

    cp "$target_dir/sip-router"   "$INSTALL_DIR/bin/sip-router"
    cp "$target_dir/sip-edge"     "$INSTALL_DIR/bin/sip-edge"
    cp "$target_dir/media-edge"   "$INSTALL_DIR/bin/media-edge"
    cp "$target_dir/api-server"   "$INSTALL_DIR/bin/api-server"
    cp "$target_dir/cdr-worker"   "$INSTALL_DIR/bin/cdr-worker"
    chmod 755 "$INSTALL_DIR/bin/"*
    chown -R "$SERVICE_USER:$SERVICE_GROUP" "$INSTALL_DIR"
}

install_config() {
    info "安装配置文件到 $CONFIG_DIR"
    mkdir -p "$CONFIG_DIR/tls"

    local config_target="$CONFIG_DIR/config.yaml"
    if [[ ! -f "$config_target" ]]; then
        cp config.yaml "$config_target"
        warn "已创建 $config_target — 启动前必须修改节点地址、数据库凭据和密钥"
    else
        info "跳过 $config_target（已存在）"
    fi

    chown -R "$SERVICE_USER:$SERVICE_GROUP" "$CONFIG_DIR"
    chmod 600 "$config_target"
}

install_systemd() {
    info "安装 systemd 服务"
    for svc in "${SERVICES[@]}"; do
        cp "deploy/systemd/${svc}.service" "/etc/systemd/system/${svc}.service"
    done
    systemctl daemon-reload

    for svc in "${SERVICES[@]}"; do
        systemctl enable "$svc"
        info "已启用 $svc"
    done
}

install_nginx() {
    if ! command -v nginx &>/dev/null; then
        warn "nginx 未安装，跳过前端部署"
        return
    fi

    info "部署前端到 nginx"
    # 构建前端
    cd web && npm ci && npm run build && cd ..
    rm -rf /usr/share/nginx/html/vos-rs
    mkdir -p /usr/share/nginx/html/vos-rs
    cp -r web/dist/* /usr/share/nginx/html/vos-rs/

    cp deploy/nginx/vos-rs.conf /etc/nginx/conf.d/vos-rs.conf
    nginx -t && systemctl reload nginx
    info "前端已部署到 /usr/share/nginx/html/vos-rs"
}

install() {
    check_root
    info "=== VOS-RS Linux 部署 ==="
    create_user
    install_binaries
    install_config
    install_systemd
    install_nginx
    info "=== 部署完成 ==="
    echo ""
    warn "下一步："
    echo "  1. 编辑统一配置: sudo vim /etc/vos-rs/config.yaml"
    echo "  2. 校验配置: make cluster-check CONFIG_FILE=/etc/vos-rs/config.yaml"
    echo "  3. 启动服务: sudo systemctl start media-edge sip-edge sip-router api-server cdr-worker"
    echo "  4. 查看状态: sudo systemctl status sip-router sip-edge media-edge"
    echo "  5. 查看日志: sudo journalctl -u sip-edge -f"
}

uninstall() {
    check_root
    info "=== 卸载 VOS-RS ==="
    for svc in "${SERVICES[@]}"; do
        systemctl stop "$svc" 2>/dev/null || true
        systemctl disable "$svc" 2>/dev/null || true
        rm -f "/etc/systemd/system/${svc}.service"
    done
    systemctl daemon-reload
    rm -rf "$INSTALL_DIR" "$CONFIG_DIR" "$LOG_DIR"
    rm -f /etc/nginx/conf.d/vos-rs.conf
    info "已卸载（数据卷未删除）"
}

status() {
    echo "=== VOS-RS 服务状态 ==="
    for svc in "${SERVICES[@]}"; do
        echo ""
        echo "--- $svc ---"
        systemctl status "$svc" --no-pager 2>/dev/null || echo "未安装"
    done
}

logs() {
    local svc="${1:-sip-edge}"
    journalctl -u "$svc" -f --no-pager
}

case "${1:-}" in
    install)   install ;;
    uninstall) uninstall ;;
    status)    status ;;
    logs)      logs "${2:-sip-edge}" ;;
    *)
        echo "用法: $0 {install|uninstall|status|logs [service]}"
        echo "  install   - 安装所有服务"
        echo "  uninstall - 卸载所有服务"
        echo "  status    - 查看服务状态"
        echo "  logs      - 查看日志 (默认 sip-edge)"
        exit 1
        ;;
esac
