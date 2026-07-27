use crate::StorageError;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

pub(crate) fn hmac_sha256_raw(key: &[u8], data: &[u8]) -> Result<Vec<u8>, StorageError> {
    let mut mac = HmacSha256::new_from_slice(key)
        .map_err(|e| StorageError::ConfigError(format!("HMAC key error: {e}")))?;
    mac.update(data);
    Ok(mac.finalize().into_bytes().to_vec())
}

pub(crate) fn percent_encode_query(value: &str) -> String {
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

pub(crate) struct AwsV4Signer<'a> {
    pub secret_key: &'a str,
    pub access_key: &'a str,
    pub region: &'a str,
    pub bucket: &'a str,
    pub host: &'a str,
}

impl<'a> AwsV4Signer<'a> {
    pub fn sign(
        &self,
        method: &str,
        full_key: &str,
        date: &str,
        date_full: &str,
        payload_hash: &str,
        content_type: &str,
    ) -> Result<String, StorageError> {
        let scope = format!("{}/{}/s3/aws4_request", date, self.region);
        let headers = format!(
            "content-type:{content_type}\nhost:{host}\nx-amz-content-sha256:{payload_hash}\nx-amz-date:{date_full}\n",
            host = self.host
        );
        let signed_headers = "content-type;host;x-amz-content-sha256;x-amz-date";
        let uri = format!("/{}/{}", self.bucket, full_key);

        let req = format!("{method}\n{uri}\n\n{headers}\n{signed_headers}\n{payload_hash}");
        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{date_full}\n{scope}\n{:x}",
            Sha256::digest(req.as_bytes())
        );

        let k_secret = format!("AWS4{}", self.secret_key);
        let k_date = hmac_sha256_raw(k_secret.as_bytes(), date.as_bytes())?;
        let k_region = hmac_sha256_raw(&k_date, self.region.as_bytes())?;
        let k_service = hmac_sha256_raw(&k_region, b"s3")?;
        let k_signing = hmac_sha256_raw(&k_service, b"aws4_request")?;
        let sig = hex::encode(hmac_sha256_raw(&k_signing, string_to_sign.as_bytes())?);

        Ok(format!(
            "AWS4-HMAC-SHA256 Credential={key}/{scope}, SignedHeaders={signed_headers}, Signature={sig}",
            key = self.access_key,
        ))
    }
}
