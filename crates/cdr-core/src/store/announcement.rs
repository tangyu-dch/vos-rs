//! 公告管理、定向投放与每用户阅读状态存储。

use crate::PostgresCdrStore;

mod model;

use model::parse_announcement;
pub use model::{Announcement, AnnouncementSummary, UpsertAnnouncementInput};

impl PostgresCdrStore {
    /// 分页列出全部公告，供管理端使用。
    pub async fn list_announcements(
        &self,
        status: Option<&str>,
        category: Option<&str>,
        query: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<(Vec<Announcement>, i64), sqlx::Error> {
        let rows = sqlx::query(
            "SELECT id, title, category, content, status, audience, audience_users, \
                    delivery_methods, scheduled_at, published_at, pinned, publisher, \
                    FALSE AS is_read, created_at, updated_at \
             FROM announcements WHERE ($1::TEXT IS NULL OR status = $1) \
               AND ($2::TEXT IS NULL OR category = $2) \
               AND ($3::TEXT IS NULL OR title ILIKE '%' || $3 || '%' OR content ILIKE '%' || $3 || '%') \
             ORDER BY pinned DESC, created_at DESC LIMIT $4 OFFSET $5",
        )
        .bind(status)
        .bind(category)
        .bind(query)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        let total = sqlx::query_scalar(
            "SELECT COUNT(*) FROM announcements WHERE ($1::TEXT IS NULL OR status = $1) \
             AND ($2::TEXT IS NULL OR category = $2) \
             AND ($3::TEXT IS NULL OR title ILIKE '%' || $3 || '%' OR content ILIKE '%' || $3 || '%')",
        )
        .bind(status)
        .bind(category)
        .bind(query)
        .fetch_one(&self.pool)
        .await?;
        Ok((rows.into_iter().map(parse_announcement).collect(), total))
    }

    /// 读取单条公告，不限制发布状态或投放范围。
    pub async fn get_announcement(&self, id: i64) -> Result<Option<Announcement>, sqlx::Error> {
        sqlx::query(
            "SELECT id, title, category, content, status, audience, audience_users, \
                    delivery_methods, scheduled_at, published_at, pinned, publisher, \
                    FALSE AS is_read, created_at, updated_at \
             FROM announcements WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map(|row| row.map(parse_announcement))
    }

    /// 创建草稿公告。
    pub async fn create_announcement(
        &self,
        input: &UpsertAnnouncementInput,
        publisher: &str,
    ) -> Result<Announcement, sqlx::Error> {
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO announcements \
               (title, category, content, audience, audience_users, delivery_methods, \
                scheduled_at, pinned, publisher) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id",
        )
        .bind(&input.title)
        .bind(&input.category)
        .bind(&input.content)
        .bind(&input.audience)
        .bind(&input.audience_users)
        .bind(&input.delivery_methods)
        .bind(input.scheduled_at)
        .bind(input.pinned)
        .bind(publisher)
        .fetch_one(&self.pool)
        .await?;
        self.get_announcement(id)
            .await?
            .ok_or(sqlx::Error::RowNotFound)
    }

    /// 更新公告内容和投放设置，不修改发布人与发布状态。
    pub async fn update_announcement(
        &self,
        id: i64,
        input: &UpsertAnnouncementInput,
    ) -> Result<Option<Announcement>, sqlx::Error> {
        let updated_id = sqlx::query_scalar(
            "UPDATE announcements SET title = $2, category = $3, content = $4, audience = $5, \
                    audience_users = $6, delivery_methods = $7, scheduled_at = $8, pinned = $9, \
                    updated_at = now() WHERE id = $1 RETURNING id",
        )
        .bind(id)
        .bind(&input.title)
        .bind(&input.category)
        .bind(&input.content)
        .bind(&input.audience)
        .bind(&input.audience_users)
        .bind(&input.delivery_methods)
        .bind(input.scheduled_at)
        .bind(input.pinned)
        .fetch_optional(&self.pool)
        .await?;
        match updated_id {
            Some(id) => self.get_announcement(id).await,
            None => Ok(None),
        }
    }

    /// 发布公告；重复发布保持原发布时间。
    pub async fn publish_announcement(
        &self,
        id: i64,
        publisher: &str,
    ) -> Result<Option<Announcement>, sqlx::Error> {
        let id = sqlx::query_scalar(
            "UPDATE announcements SET status = 'published', published_at = COALESCE(published_at, now()), \
                    publisher = $2, updated_at = now() WHERE id = $1 RETURNING id",
        )
        .bind(id)
        .bind(publisher)
        .fetch_optional(&self.pool)
        .await?;
        match id {
            Some(id) => self.get_announcement(id).await,
            None => Ok(None),
        }
    }

    /// 删除公告及其阅读回执。
    pub async fn delete_announcement(&self, id: i64) -> Result<bool, sqlx::Error> {
        sqlx::query("DELETE FROM announcements WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map(|result| result.rows_affected() > 0)
    }

    /// 分页列出当前用户已发布且已到投放时间的公告。
    pub async fn list_visible_announcements(
        &self,
        operator: &str,
        unread_only: bool,
        query: Option<&str>,
        category: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<(Vec<Announcement>, AnnouncementSummary), sqlx::Error> {
        let rows = sqlx::query(
            "SELECT a.id, a.title, a.category, a.content, a.status, a.audience, a.audience_users, \
                    a.delivery_methods, a.scheduled_at, a.published_at, a.pinned, a.publisher, \
                    (r.announcement_id IS NOT NULL) AS is_read, a.created_at, a.updated_at \
             FROM announcements a LEFT JOIN announcement_reads r \
               ON r.announcement_id = a.id AND r.operator = $1 \
             WHERE a.status = 'published' AND (a.scheduled_at IS NULL OR a.scheduled_at <= now()) \
               AND (a.audience = 'all' OR $1 = ANY(a.audience_users)) \
               AND (NOT $2 OR r.announcement_id IS NULL) \
               AND ($3::TEXT IS NULL OR a.title ILIKE '%' || $3 || '%' OR a.content ILIKE '%' || $3 || '%') \
               AND ($4::TEXT IS NULL OR a.category = $4) \
             ORDER BY a.pinned DESC, COALESCE(a.scheduled_at, a.published_at) DESC, a.created_at DESC \
             LIMIT $5 OFFSET $6",
        )
        .bind(operator)
        .bind(unread_only)
        .bind(query)
        .bind(category)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        let counts: (i64, i64) = sqlx::query_as(
            "SELECT COUNT(*), COUNT(*) FILTER (WHERE r.announcement_id IS NULL) \
             FROM announcements a LEFT JOIN announcement_reads r \
               ON r.announcement_id = a.id AND r.operator = $1 \
             WHERE a.status = 'published' AND (a.scheduled_at IS NULL OR a.scheduled_at <= now()) \
               AND (a.audience = 'all' OR $1 = ANY(a.audience_users)) \
               AND ($2::TEXT IS NULL OR a.title ILIKE '%' || $2 || '%' OR a.content ILIKE '%' || $2 || '%') \
               AND ($3::TEXT IS NULL OR a.category = $3)",
        )
        .bind(operator)
        .bind(query)
        .bind(category)
        .fetch_one(&self.pool)
        .await?;
        Ok((
            rows.into_iter().map(parse_announcement).collect(),
            AnnouncementSummary {
                total: counts.0,
                unread: counts.1,
            },
        ))
    }

    /// 读取当前用户可见的单条公告。
    pub async fn get_visible_announcement(
        &self,
        id: i64,
        operator: &str,
    ) -> Result<Option<Announcement>, sqlx::Error> {
        sqlx::query(
            "SELECT a.id, a.title, a.category, a.content, a.status, a.audience, a.audience_users, \
                    a.delivery_methods, a.scheduled_at, a.published_at, a.pinned, a.publisher, \
                    (r.announcement_id IS NOT NULL) AS is_read, a.created_at, a.updated_at \
             FROM announcements a LEFT JOIN announcement_reads r \
               ON r.announcement_id = a.id AND r.operator = $2 \
             WHERE a.id = $1 AND a.status = 'published' \
               AND (a.scheduled_at IS NULL OR a.scheduled_at <= now()) \
               AND (a.audience = 'all' OR $2 = ANY(a.audience_users))",
        )
        .bind(id)
        .bind(operator)
        .fetch_optional(&self.pool)
        .await
        .map(|row| row.map(parse_announcement))
    }

    /// 将当前用户可见的公告标记为已读。
    pub async fn mark_announcement_read(
        &self,
        id: i64,
        operator: &str,
    ) -> Result<bool, sqlx::Error> {
        let result = sqlx::query(
            "INSERT INTO announcement_reads (announcement_id, operator) \
             SELECT id, $2 FROM announcements WHERE id = $1 AND status = 'published' \
               AND (scheduled_at IS NULL OR scheduled_at <= now()) \
               AND (audience = 'all' OR $2 = ANY(audience_users)) \
             ON CONFLICT (announcement_id, operator) DO NOTHING",
        )
        .bind(id)
        .bind(operator)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() > 0 {
            return Ok(true);
        }
        Ok(self.get_visible_announcement(id, operator).await?.is_some())
    }
}
