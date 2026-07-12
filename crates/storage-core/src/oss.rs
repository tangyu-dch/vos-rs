use async_trait::async_trait;
use bytes::Bytes;
use chrono::Utc;
use reqwest::Client;
use sha2::{Digest, Sha256};

use crate::{FileInfo, StorageBackend, StorageError};

/// OSS 兼容对象存储后端。
/// 支持阿里云 OSS、MinIO 等兼容 S3/REST 协议的存储。
pub struct OssStorage {
    client: Client,
    endpoint: String,
    bucket: String,
    access_key: String,
    secret_key: String,
    key_prefix: String,
}

impl OssStorage {
    pub fn new(
        endpoint: &str,
        bucket: &str,
        access_key: &str,
        secret_key: &str,
        key_prefix: String,
    ) -> Result<Self, StorageError> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .http1_only()
            .build()
            .map_err(|e| StorageError::ConfigError(format!("创建 HTTP 客户端失败: {e}")))?;

        Ok(Self {
            client,
            endpoint: endpoint.trim_end_matches('/').to_string(),
            bucket: bucket.to_string(),
            access_key: access_key.to_string(),
            secret_key: secret_key.to_string(),
            key_prefix,
        })
    }

    fn full_key(&self, key: &str) -> String {
        if self.key_prefix.is_empty() {
            key.to_string()
        } else {
            let prefix = self.key_prefix.trim_end_matches('/');
            if key.is_empty() {
                prefix.to_string()
            } else if key.starts_with('/') {
                format!("{prefix}{key}")
            } else {
                format!("{prefix}/{key}")
            }
        }
    }

    fn object_url(&self, key: &str) -> String {
        format!("{}/{}/{}", self.endpoint, self.bucket, self.full_key(key))
    }

    fn host(&self) -> String {
        let without_proto = self
            .endpoint
            .trim_start_matches("https://")
            .trim_start_matches("http://");
        if without_proto.contains("aliyuncs.com") || without_proto.contains("amazonaws.com") {
            format!("{}.{}", self.bucket, without_proto)
        } else {
            without_proto.to_string()
        }
    }

    fn sign_v4(
        &self,
        method: &str,
        full_key: &str,
        date: &str,
        date_full: &str,
        payload_hash: &str,
        content_type: &str,
    ) -> String {
        let region = std::env::var("VOS_RS_OSS_REGION").unwrap_or_else(|_| "us-east-1".to_string());
        let credential_scope = format!("{}/{}/s3/aws4_request", date, region);
        let host = self.host();

        let canonical_headers = format!(
            "content-type:{content_type}\nhost:{host}\nx-amz-content-sha256:{payload_hash}\nx-amz-date:{date_full}\n"
        );
        let signed_headers = "content-type;host;x-amz-content-sha256;x-amz-date";
        // RustFS 默认使用 path-style：/{bucket}/{object-key}。
        let canonical_uri = format!("/{}/{}", self.bucket, full_key);

        let canonical_request = format!(
            "{method}\n{canonical_uri}\n\n{canonical_headers}\n{signed_headers}\n{payload_hash}"
        );

        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{date_full}\n{credential_scope}\n{:x}",
            Sha256::digest(canonical_request.as_bytes())
        );

        let k_secret = format!("AWS4{}", self.secret_key);
        let k_date = hmac_sha256_raw(k_secret.as_bytes(), date.as_bytes());
        let k_region = hmac_sha256_raw(&k_date, region.as_bytes());
        let k_service = hmac_sha256_raw(&k_region, b"s3");
        let k_signing = hmac_sha256_raw(&k_service, b"aws4_request");
        let signature = hex::encode(hmac_sha256_raw(&k_signing, string_to_sign.as_bytes()));

        format!(
            "AWS4-HMAC-SHA256 Credential={access_key}/{scope}, SignedHeaders={signed}, Signature={sig}",
            access_key = self.access_key,
            scope = credential_scope,
            signed = signed_headers,
            sig = signature,
        )
    }

    async fn do_request(
        &self,
        method: reqwest::Method,
        key: &str,
        body: Option<Bytes>,
    ) -> Result<reqwest::Response, StorageError> {
        let url = self.object_url(key);
        let date = Utc::now().format("%Y%m%d").to_string();
        let date_full = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
        let payload_hash = body
            .as_ref()
            .map(|b| format!("{:x}", Sha256::digest(b)))
            .unwrap_or_else(|| {
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string()
            });
        let authorization = self.sign_v4(
            method.as_str(),
            &self.full_key(key),
            &date,
            &date_full,
            &payload_hash,
            "application/octet-stream",
        );

        let mut req = self
            .client
            .request(method, &url)
            .header("Authorization", &authorization)
            .header("x-amz-date", &date_full)
            .header("x-amz-content-sha256", &payload_hash)
            .header("Content-Type", "application/octet-stream");

        if let Some(body) = body {
            req = req.body(body);
        }

        Ok(req.send().await?)
    }
}

fn hmac_sha256_raw(key: &[u8], data: &[u8]) -> Vec<u8> {
    use hmac::{Hmac, Mac};
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC 可接受任意长度 key");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

/// 按 RFC 3986 规则编码 S3 查询参数。
///
/// 签名计算和实际请求必须使用完全相同的编码结果；不能直接把对象前缀
/// 拼接到 URL，否则前缀中的 `/`、空格或非 ASCII 字符会导致签名不一致。
fn percent_encode_query(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push('%');
            encoded.push_str(&format!("{byte:02X}"));
        }
    }
    encoded
}

#[async_trait]
impl StorageBackend for OssStorage {
    async fn put(
        &self,
        key: &str,
        data: Bytes,
        content_type: Option<&str>,
    ) -> Result<(), StorageError> {
        let url = self.object_url(key);
        let date = Utc::now().format("%Y%m%d").to_string();
        let date_full = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
        let payload_hash = format!("{:x}", Sha256::digest(&data));
        let ct = content_type.unwrap_or("application/octet-stream");
        let authorization = self.sign_v4(
            "PUT",
            &self.full_key(key),
            &date,
            &date_full,
            &payload_hash,
            ct,
        );

        let resp = self
            .client
            .put(&url)
            .header("Authorization", &authorization)
            .header("x-amz-date", &date_full)
            .header("x-amz-content-sha256", &payload_hash)
            .header("Content-Type", ct)
            .body(data)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(StorageError::ConfigError(format!(
                "OSS PUT 失败: {status} - {body}"
            )));
        }
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Bytes, StorageError> {
        let resp = self.do_request(reqwest::Method::GET, key, None).await?;

        if resp.status() == 404 {
            return Err(StorageError::NotFound(key.to_string()));
        }
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(StorageError::ConfigError(format!(
                "OSS GET 失败: {status} - {body}"
            )));
        }

        Ok(resp.bytes().await?)
    }

    async fn list(&self, prefix: &str) -> Result<Vec<FileInfo>, StorageError> {
        let full_prefix = self.full_key(prefix);
        let date = Utc::now().format("%Y%m%d").to_string();
        let date_full = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
        let empty_hash = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

        let region = std::env::var("VOS_RS_OSS_REGION").unwrap_or_else(|_| "us-east-1".to_string());
        let credential_scope = format!("{}/{}/s3/aws4_request", date, region);
        let host = self.host();
        let encoded_prefix = percent_encode_query(&full_prefix);

        // S3 ListObjects: path is /{bucket}/ with query prefix=...
        let canonical_uri = format!("/{}/", self.bucket);
        let canonical_query = format!("prefix={encoded_prefix}");
        let canonical_headers = format!(
            "content-type:application/octet-stream\nhost:{host}\nx-amz-content-sha256:{empty_hash}\nx-amz-date:{date_full}\n"
        );
        let signed_headers = "content-type;host;x-amz-content-sha256;x-amz-date";

        let canonical_request = format!(
            "GET\n{canonical_uri}\n{canonical_query}\n{canonical_headers}\n{signed_headers}\n{empty_hash}"
        );

        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{date_full}\n{credential_scope}\n{:x}",
            Sha256::digest(canonical_request.as_bytes())
        );

        let k_secret = format!("AWS4{}", self.secret_key);
        let k_date = hmac_sha256_raw(k_secret.as_bytes(), date.as_bytes());
        let k_region = hmac_sha256_raw(&k_date, region.as_bytes());
        let k_service = hmac_sha256_raw(&k_region, b"s3");
        let k_signing = hmac_sha256_raw(&k_service, b"aws4_request");
        let signature = hex::encode(hmac_sha256_raw(&k_signing, string_to_sign.as_bytes()));

        let authorization = format!(
            "AWS4-HMAC-SHA256 Credential={access_key}/{scope}, SignedHeaders={signed}, Signature={sig}",
            access_key = self.access_key,
            scope = credential_scope,
            signed = signed_headers,
            sig = signature,
        );

        let url = format!("{}/{}/?prefix={encoded_prefix}", self.endpoint, self.bucket);
        let response = self
            .client
            .get(&url)
            .header("Authorization", &authorization)
            .header("x-amz-date", &date_full)
            .header("x-amz-content-sha256", empty_hash)
            .header("Content-Type", "application/octet-stream")
            .send()
            .await?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            tracing::warn!(status = %status, url = %url, "OSS ListObjects 失败");
            return Err(StorageError::ConfigError(format!(
                "OSS ListObjects 失败: {status} - {body}"
            )));
        }

        let mut results = Vec::new();
        let mut rest = body.as_str();
        while let Some(start) = rest.find("<Key>") {
            rest = &rest[start + 5..];
            if let Some(end) = rest.find("</Key>") {
                let key = &rest[..end];
                let key = key.strip_prefix(&self.key_prefix).unwrap_or(key);
                results.push(FileInfo {
                    key: key.to_string(),
                    size: 0,
                    content_type: None,
                    last_modified: None,
                });
                rest = &rest[end + 6..];
            } else {
                break;
            }
        }
        Ok(results)
    }

    async fn exists(&self, key: &str) -> Result<bool, StorageError> {
        let resp = self.do_request(reqwest::Method::HEAD, key, None).await?;
        Ok(resp.status().is_success())
    }

    async fn delete(&self, key: &str) -> Result<(), StorageError> {
        let resp = self.do_request(reqwest::Method::DELETE, key, None).await?;

        if !resp.status().is_success() && resp.status().as_u16() != 404 {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(StorageError::ConfigError(format!(
                "OSS DELETE 失败: {status} - {body}"
            )));
        }
        Ok(())
    }

    async fn presign_get(&self, key: &str, expires_secs: u64) -> Result<String, StorageError> {
        let now = Utc::now();
        let date = now.format("%Y%m%d").to_string();
        let region =
            std::env::var("VOS_RS_OSS_REGION").unwrap_or_else(|_| "cn-hangzhou".to_string());

        let full_key = self.full_key(key);
        let credential_scope = format!("{}/{}/s3/aws4_request", date, region);
        let host = self.host();
        let empty_hash = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

        let canonical_uri = format!("/{}/{}", self.bucket, full_key);
        let canonical_request = format!(
            "GET\n{canonical_uri}\n\ncontent-type:application/octet-stream\nhost:{host}\nx-amz-content-sha256:{empty_hash}\n\ncontent-type;host;x-amz-content-sha256\n{empty_hash}"
        );

        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{date}T000000Z\n{credential_scope}\n{:x}",
            Sha256::digest(canonical_request.as_bytes())
        );

        let k_secret = format!("AWS4{}", self.secret_key);
        let k_date = hmac_sha256_raw(k_secret.as_bytes(), date.as_bytes());
        let k_region = hmac_sha256_raw(&k_date, region.as_bytes());
        let k_service = hmac_sha256_raw(&k_region, b"s3");
        let k_signing = hmac_sha256_raw(&k_service, b"aws4_request");
        let signature = hex::encode(hmac_sha256_raw(&k_signing, string_to_sign.as_bytes()));

        let expiry = now.timestamp() + expires_secs as i64;
        let sep = if self.endpoint.contains('?') {
            '&'
        } else {
            '?'
        };

        Ok(format!(
            "{}/{}/{full_key}{sep}X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential={}/{}/s3/aws4_request&X-Amz-Date={date}T000000Z&X-Amz-Expires={expiry}&X-Amz-SignedHeaders=content-type;host;x-amz-content-sha256&X-Amz-Signature={signature}",
            self.endpoint,
            self.bucket,
            self.access_key,
            region,
        ))
    }

    fn backend_name(&self) -> &str {
        "oss"
    }
}
