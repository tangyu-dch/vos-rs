use axum::{
    body::Bytes,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::IntoResponse,
};

pub(crate) fn build_audio_response(
    bytes: Bytes,
    prefix: &str,
    requested_range: Option<&str>,
) -> Result<axum::response::Response, (StatusCode, String)> {
    let selected_range = match parse_byte_range(requested_range, bytes.len()) {
        Ok(range) => range,
        Err(()) => return Ok(range_not_satisfiable(bytes.len())),
    };
    let (status, body, content_range) = if let Some((start, end)) = selected_range {
        (
            StatusCode::PARTIAL_CONTENT,
            bytes.slice(start..=end),
            Some(format!("bytes {start}-{end}/{}", bytes.len())),
        )
    } else {
        (StatusCode::OK, bytes, None)
    };
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("audio/wav"));
    headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("inline; filename=\"{prefix}.wav\""))
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "录音文件名无效".into()))?,
    );
    headers.insert(
        axum::http::HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    if let Some(value) = content_range {
        headers.insert(
            header::CONTENT_RANGE,
            HeaderValue::from_str(&value)
                .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "音频范围无效".into()))?,
        );
    }
    headers.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&body.len().to_string())
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "音频长度无效".into()))?,
    );
    Ok((status, headers, body).into_response())
}

pub(crate) fn parse_byte_range(
    value: Option<&str>,
    length: usize,
) -> Result<Option<(usize, usize)>, ()> {
    let Some(value) = value else {
        return Ok(None);
    };
    let spec = value.strip_prefix("bytes=").ok_or(())?;
    if spec.contains(',') || length == 0 {
        return Err(());
    }
    let (start, end) = spec.split_once('-').ok_or(())?;
    if start.is_empty() {
        let suffix = end.parse::<usize>().map_err(|_| ())?;
        if suffix == 0 {
            return Err(());
        }
        let size = suffix.min(length);
        return Ok(Some((length - size, length - 1)));
    }
    let start = start.parse::<usize>().map_err(|_| ())?;
    if start >= length {
        return Err(());
    }
    let end = if end.is_empty() {
        length - 1
    } else {
        end.parse::<usize>().map_err(|_| ())?.min(length - 1)
    };
    if end < start {
        return Err(());
    }
    Ok(Some((start, end)))
}

pub(crate) fn range_not_satisfiable(length: usize) -> axum::response::Response {
    let mut headers = HeaderMap::new();
    headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    if let Ok(value) = HeaderValue::from_str(&format!("bytes */{length}")) {
        headers.insert(header::CONTENT_RANGE, value);
    }
    (StatusCode::RANGE_NOT_SATISFIABLE, headers).into_response()
}

pub(crate) fn recording_not_available_response() -> axum::response::Response {
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::HeaderName::from_static("x-recording-status"),
        HeaderValue::from_static("not-generated"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    (StatusCode::NO_CONTENT, headers).into_response()
}

#[cfg(test)]
mod tests {
    use super::{parse_byte_range, range_not_satisfiable, recording_not_available_response};
    use axum::http::{header, HeaderValue, StatusCode};

    #[test]
    fn parses_browser_byte_ranges() {
        assert_eq!(parse_byte_range(None, 100), Ok(None));
        assert_eq!(
            parse_byte_range(Some("bytes=10-19"), 100),
            Ok(Some((10, 19)))
        );
        assert_eq!(parse_byte_range(Some("bytes=90-"), 100), Ok(Some((90, 99))));
        assert_eq!(parse_byte_range(Some("bytes=-10"), 100), Ok(Some((90, 99))));
        assert_eq!(
            parse_byte_range(Some("bytes=90-200"), 100),
            Ok(Some((90, 99)))
        );
    }

    #[test]
    fn missing_recording_is_a_normal_empty_response() {
        let response = recording_not_available_response();

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert_eq!(
            response.headers().get("x-recording-status"),
            Some(&HeaderValue::from_static("not-generated"))
        );
    }

    #[test]
    fn rejects_invalid_or_multiple_ranges() {
        assert!(parse_byte_range(Some("bytes=100-"), 100).is_err());
        assert!(parse_byte_range(Some("bytes=20-10"), 100).is_err());
        assert!(parse_byte_range(Some("bytes=0-1,4-5"), 100).is_err());
        assert!(parse_byte_range(Some("items=0-1"), 100).is_err());
    }

    #[test]
    fn unsatisfied_range_advertises_complete_length() {
        let response = range_not_satisfiable(1234);
        assert_eq!(response.status(), StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(
            response.headers().get(header::CONTENT_RANGE),
            Some(&HeaderValue::from_static("bytes */1234"))
        );
        assert_eq!(
            response.headers().get(header::ACCEPT_RANGES),
            Some(&HeaderValue::from_static("bytes"))
        );
    }
}
