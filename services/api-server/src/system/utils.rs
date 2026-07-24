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
    csv_data.push('\u{feff}');

    // Headers
    let header_line = headers
        .iter()
        .map(|h| csv_quote(h))
        .collect::<Vec<_>>()
        .join(",");
    csv_data.push_str(&header_line);
    csv_data.push('\n');

    // Rows
    for row in rows {
        let row_line = row
            .iter()
            .map(|col| csv_quote(col))
            .collect::<Vec<_>>()
            .join(",");
        csv_data.push_str(&row_line);
        csv_data.push('\n');
    }

    Response::builder()
        .status(StatusCode::OK)
        .header(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/csv; charset=utf-8"),
        )
        .header(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_str(&format!("attachment; filename=\"{}\"", filename))
                .unwrap_or_else(|_| {
                    HeaderValue::from_static("attachment; filename=\"export.csv\"")
                }),
        )
        .body(Body::from(csv_data))
        .unwrap_or_else(|_| {
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to generate CSV").into_response()
        })
}

pub fn parse_csv(content: &str) -> Vec<Vec<String>> {
    let mut result = Vec::new();
    let normalized = content.replace("\r\n", "\n").replace('\r', "\n");
    for line in normalized.split('\n') {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut row = Vec::new();
        let mut current = String::new();
        let mut in_quotes = false;
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c == '"' {
                if in_quotes && i + 1 < chars.len() && chars[i + 1] == '"' {
                    current.push('"');
                    i += 2;
                    continue;
                } else {
                    in_quotes = !in_quotes;
                }
            } else if c == ',' && !in_quotes {
                row.push(current.trim().to_string());
                current = String::new();
            } else {
                current.push(c);
            }
            i += 1;
        }
        row.push(current.trim().to_string());
        result.push(row);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_csv_simple() {
        let data = "col1,col2,col3\nval1,val2,val3";
        let parsed = parse_csv(data);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0], vec!["col1", "col2", "col3"]);
        assert_eq!(parsed[1], vec!["val1", "val2", "val3"]);
    }

    #[test]
    fn test_parse_csv_with_quotes() {
        let data = "col1,\"col2,with,commas\",col3\n\"val1\"\"with\"\"quotes\",val2,val3";
        let parsed = parse_csv(data);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0], vec!["col1", "col2,with,commas", "col3"]);
        assert_eq!(parsed[1], vec!["val1\"with\"quotes", "val2", "val3"]);
    }
}
