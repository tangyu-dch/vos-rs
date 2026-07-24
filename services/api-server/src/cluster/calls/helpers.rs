use axum::http::StatusCode;

pub(super) type E = (StatusCode, String);

pub(super) fn err(e: impl std::fmt::Display) -> E {
    (StatusCode::BAD_GATEWAY, e.to_string())
}

pub(super) fn get_internal_token(token: &str) -> Result<String, E> {
    if token.is_empty() {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_secret 未配置".to_string(),
        ));
    }
    Ok(token.to_string())
}

pub(super) fn urlencoding(s: &str) -> String {
    s.as_bytes()
        .iter()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
                (*byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::urlencoding;

    #[test]
    fn path_and_query_delimiters_are_percent_encoded() {
        assert_eq!(urlencoding("a/b?c#d"), "a%2Fb%3Fc%23d");
        assert_eq!(urlencoding("通话"), "%E9%80%9A%E8%AF%9D");
    }
}
