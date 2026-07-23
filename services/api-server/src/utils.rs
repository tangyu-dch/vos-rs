use axum::{
    body::Body,
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};

pub fn csv_quote(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

pub fn to_csv_response(filename: &str, headers: &[&str], rows: &[Vec<String>]) -> Response {
    let mut csv_data = String::new();
    // UTF-8 BOM to support Chinese characters in Excel
    csv_data.push_str("\u{feff}");

    // Headers
    let header_line = headers
        .iter()
        .map(|h| csv_quote(h))
        .collect::<Vec<_>>()
        .join(",");
    csv_data.push_str(&header_line);
    csv_data.push_str("\n");

    // Rows
    for row in rows {
        let row_line = row
            .iter()
            .map(|col| csv_quote(col))
            .collect::<Vec<_>>()
            .join(",");
        csv_data.push_str(&row_line);
        csv_data.push_str("\n");
    }

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, HeaderValue::from_static("text/csv; charset=utf-8"))
        .header(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_str(&format!("attachment; filename=\"{}\"", filename))
                .unwrap_or_else(|_| HeaderValue::from_static("attachment; filename=\"export.csv\"")),
        )
        .body(Body::from(csv_data))
        .unwrap_or_else(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Failed to generate CSV").into_response())
}
