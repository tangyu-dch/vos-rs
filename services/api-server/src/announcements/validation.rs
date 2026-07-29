//! 公告输入校验与标准化。

use serde::Deserialize;
use time::OffsetDateTime;

use crate::ApiError;

/// 公告创建或更新请求。
#[derive(Debug, Deserialize)]
pub(crate) struct AnnouncementPayload {
    title: String,
    category: String,
    content: String,
    audience: String,
    #[serde(default)]
    audience_users: Vec<String>,
    #[serde(default = "default_delivery_methods")]
    delivery_methods: Vec<String>,
    #[serde(default, with = "time::serde::rfc3339::option")]
    scheduled_at: Option<OffsetDateTime>,
    #[serde(default)]
    pinned: bool,
}

pub(super) fn validate_payload(
    payload: AnnouncementPayload,
) -> Result<cdr_core::UpsertAnnouncementInput, ApiError> {
    let title = required_text("标题", payload.title, 200)?;
    let category = required_text("分类", payload.category, 50)?;
    let content = required_text("内容", payload.content, 50_000)?;
    if !matches!(payload.audience.as_str(), "all" | "specified") {
        return Err(ApiError::bad_request(
            "参数无效：投放范围仅支持 all 或 specified",
        ));
    }
    let audience_users = normalized_unique(payload.audience_users);
    if payload.audience == "specified" && audience_users.is_empty() {
        return Err(ApiError::bad_request("参数无效：指定用户投放必须选择用户"));
    }
    let delivery_methods = normalized_unique(payload.delivery_methods);
    if delivery_methods.is_empty()
        || delivery_methods
            .iter()
            .any(|method| !matches!(method.as_str(), "system" | "popup"))
    {
        return Err(ApiError::bad_request(
            "参数无效：投放方式仅支持 system 或 popup",
        ));
    }
    let is_all_audience = payload.audience == "all";
    Ok(cdr_core::UpsertAnnouncementInput {
        title,
        category,
        content,
        audience: payload.audience,
        audience_users: if is_all_audience {
            Vec::new()
        } else {
            audience_users
        },
        delivery_methods,
        scheduled_at: payload.scheduled_at,
        pinned: payload.pinned,
    })
}

fn required_text(name: &str, value: String, max_chars: usize) -> Result<String, ApiError> {
    let value = value.trim().to_string();
    if value.is_empty() || value.chars().count() > max_chars {
        return Err(ApiError::bad_request(format!(
            "参数无效：{name}不能为空且不能超过 {max_chars} 个字符"
        )));
    }
    Ok(value)
}

fn normalized_unique(values: Vec<String>) -> Vec<String> {
    let mut values: Vec<String> = values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect();
    values.sort();
    values.dedup();
    values
}

fn default_delivery_methods() -> Vec<String> {
    vec!["system".to_string()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(audience: &str, users: Vec<&str>, methods: Vec<&str>) -> AnnouncementPayload {
        AnnouncementPayload {
            title: "系统维护".to_string(),
            category: "system".to_string(),
            content: "维护通知".to_string(),
            audience: audience.to_string(),
            audience_users: users.into_iter().map(str::to_string).collect(),
            delivery_methods: methods.into_iter().map(str::to_string).collect(),
            scheduled_at: None,
            pinned: false,
        }
    }

    #[test]
    fn specified_audience_requires_users() {
        assert!(validate_payload(payload("specified", Vec::new(), vec!["system"])).is_err());
        assert!(validate_payload(payload("specified", vec!["alice"], vec!["system"])).is_ok());
    }

    #[test]
    fn delivery_methods_are_restricted_and_deduplicated() {
        assert!(validate_payload(payload("all", Vec::new(), vec!["email"])).is_err());
        let input = validate_payload(payload("all", Vec::new(), vec!["popup", "popup"]))
            .expect("合法投放方式");
        assert_eq!(input.delivery_methods, vec!["popup"]);
    }
}
