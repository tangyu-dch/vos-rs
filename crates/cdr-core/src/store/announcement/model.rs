//! 公告存储模型与数据库行转换。

use serde::Serialize;
use sqlx::Row;
use time::OffsetDateTime;

/// 公告完整记录。
#[derive(Debug, Clone, Serialize)]
pub struct Announcement {
    pub id: i64,
    pub title: String,
    pub category: String,
    pub content: String,
    pub status: String,
    pub audience: String,
    pub audience_users: Vec<String>,
    pub delivery_methods: Vec<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub scheduled_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub published_at: Option<OffsetDateTime>,
    pub pinned: bool,
    pub publisher: String,
    pub is_read: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

/// 公告创建和更新的持久化输入。
#[derive(Debug, Clone)]
pub struct UpsertAnnouncementInput {
    pub title: String,
    pub category: String,
    pub content: String,
    pub audience: String,
    pub audience_users: Vec<String>,
    pub delivery_methods: Vec<String>,
    pub scheduled_at: Option<OffsetDateTime>,
    pub pinned: bool,
}

/// 公告列表摘要。
#[derive(Debug, Clone, Copy, Serialize)]
pub struct AnnouncementSummary {
    pub total: i64,
    pub unread: i64,
}

pub(super) fn parse_announcement(row: sqlx::postgres::PgRow) -> Announcement {
    Announcement {
        id: row.get("id"),
        title: row.get("title"),
        category: row.get("category"),
        content: row.get("content"),
        status: row.get("status"),
        audience: row.get("audience"),
        audience_users: row.get("audience_users"),
        delivery_methods: row.get("delivery_methods"),
        scheduled_at: row.get("scheduled_at"),
        published_at: row.get("published_at"),
        pinned: row.get("pinned"),
        publisher: row.get("publisher"),
        is_read: row.get("is_read"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}
