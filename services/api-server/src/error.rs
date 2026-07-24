use axum::{http::StatusCode, response::IntoResponse, Json};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub(crate) struct ApiError {
    pub(crate) error: String,
}

impl ApiError {
    pub(crate) fn internal(msg: impl Into<String>) -> Self {
        Self { error: msg.into() }
    }

    pub(crate) fn unauthorized(msg: impl Into<String>) -> Self {
        Self { error: msg.into() }
    }

    pub(crate) fn forbidden(msg: impl Into<String>) -> Self {
        Self { error: msg.into() }
    }

    /// 404：依赖 `IntoResponse` 通过 "不存在" 关键字识别状态码
    pub(crate) fn not_found(msg: impl Into<String>) -> Self {
        Self { error: msg.into() }
    }

    /// 400：依赖 `IntoResponse` 通过 "参数无效" 关键字识别状态码
    pub(crate) fn bad_request(msg: impl Into<String>) -> Self {
        Self { error: msg.into() }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let is_unauthorized = self.error.contains("用户名或密码错误")
            || self.error.contains("缺少凭证")
            || self.error.contains("凭证格式不正确")
            || self.error.contains("无效 Token");

        let is_forbidden = self.error.contains("越权") || self.error.contains("无权");

        let status = if is_unauthorized {
            StatusCode::UNAUTHORIZED
        } else if is_forbidden {
            StatusCode::FORBIDDEN
        } else if self.error.contains("参数无效") {
            StatusCode::BAD_REQUEST
        } else if self.error.contains("不存在") {
            StatusCode::NOT_FOUND
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };
        (status, Json(self)).into_response()
    }
}
