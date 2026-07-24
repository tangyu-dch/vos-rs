#!/usr/bin/env bash
# ==============================================================================
# vos-rs - SIP TLS & HTTPS 自签名证书生成工具脚本
# 用途：为开箱启用的 SIP TLS (SIPS) 与 HTTPS API 提供一键私钥与 CA 自签名证书
# 输出目录：./deploy/tls/
# ==============================================================================

set -euo pipefail

CERT_DIR="./deploy/tls"
DAYS=3650
DOMAIN="vos-rs.local"

echo "🔐 开始为 vos-rs 生成 TLS 私钥与证书..."
mkdir -p "${CERT_DIR}"

# 1. 生成根 CA 私钥与证书
echo "1️⃣ 生成 CA 根私钥与 CA 证书..."
openssl genrsa -out "${CERT_DIR}/ca.key" 4096
openssl req -new -x509 -days "${DAYS}" -key "${CERT_DIR}/ca.key" -out "${CERT_DIR}/ca.crt" \
  -subj "/C=CN/ST=Shanghai/L=Shanghai/O=vos-rs/OU=DevOps/CN=vos-rs Root CA"

# 2. 生成服务器私钥与 CSR
echo "2️⃣ 生成服务器私钥与签名请求 (CSR)..."
openssl genrsa -out "${CERT_DIR}/server.key" 2048
openssl req -new -key "${CERT_DIR}/server.key" -out "${CERT_DIR}/server.csr" \
  -subj "/C=CN/ST=Shanghai/L=Shanghai/O=vos-rs/OU=VoIP/CN=${DOMAIN}"

# 3. 构造 SAN 扩展文件 (支持 IP 与域名)
cat > "${CERT_DIR}/san.ext" <<EOF
authorityKeyIdentifier=keyid,issuer
basicConstraints=CA:FALSE
keyUsage = digitalSignature, nonRepudiation, keyEncipherment, dataEncipherment
subjectAltName = @alt_names

[alt_names]
DNS.1 = ${DOMAIN}
DNS.2 = localhost
IP.1 = 127.0.0.1
IP.2 = 0.0.0.0
EOF

# 4. 使用 CA 签发服务器证书
echo "3️⃣ 签发服务器证书 server.crt..."
openssl x509 -req -in "${CERT_DIR}/server.csr" \
  -CA "${CERT_DIR}/ca.crt" -CAkey "${CERT_DIR}/ca.key" -CAcreateserial \
  -out "${CERT_DIR}/server.crt" -days "${DAYS}" -sha256 -extfile "${CERT_DIR}/san.ext"

# 5. 设置最严权限保护私钥
chmod 600 "${CERT_DIR}/*.key" || true

echo "✅ TLS 证书生成成功！"
echo "  - CA 证书:     ${CERT_DIR}/ca.crt"
echo "  - 服务器证书: ${CERT_DIR}/server.crt"
echo "  - 服务器私钥: ${CERT_DIR}/server.key"
