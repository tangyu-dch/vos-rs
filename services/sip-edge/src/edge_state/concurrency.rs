use crate::edge_state::EdgeState;
use sip_core::SipRequest;

impl EdgeState {
    /// 获取指定用户当前活跃并发通话数（O(1)）
    pub(crate) fn user_concurrent_count(&self, username: &str) -> u32 {
        self.user_concurrency.get(username).map_or(0, |c| *c)
    }

    /// INVITE 成功写入 inbound_transactions 后，递增该用户的并发计数
    pub(crate) fn increment_user_concurrency(&self, username: &str) {
        self.user_concurrency
            .entry(username.to_string())
            .and_modify(|c| *c += 1)
            .or_insert(1);
    }

    /// BYE/CANCEL/超时清理时递减用户并发计数（防止下溢）
    pub(crate) fn decrement_user_concurrency(&self, username: &str) {
        // remove_if 在同一分片锁内完成递减和删除，避免“先释放锁再 remove”
        // 导致并发 INVITE 刚加上的计数被误删。
        if let dashmap::mapref::entry::Entry::Occupied(mut entry) =
            self.user_concurrency.entry(username.to_string())
        {
            if *entry.get() <= 1 {
                entry.remove();
            } else {
                *entry.get_mut() -= 1;
            }
        }
    }

    /// 从 SIP 请求的 From 头中提取用户名
    pub(crate) fn username_from_request(request: &SipRequest) -> Option<String> {
        let from = request.headers.get("from")?;
        let s = from.as_str();
        let start = s.find("sip:").map(|i| i + 4)?;
        let end = s[start..].find('@')?;
        Some(s[start..start + end].to_string())
    }

    /// 从 SIP 请求的 From 头中提取域名作为租户标识
    pub(crate) fn domain_from_request(request: &SipRequest) -> Option<String> {
        let from = request.headers.get("from")?;
        let s = from.as_str();
        let start = s.find("sip:").map(|i| i + 4)?;
        let rest = &s[start..];
        let at_pos = rest.find('@')?;
        let domain_part = &rest[at_pos + 1..];
        let end_pos = domain_part
            .find(|c: char| !c.is_alphanumeric() && c != '.' && c != '-')
            .unwrap_or(domain_part.len());
        Some(domain_part[..end_pos].to_string())
    }
}
