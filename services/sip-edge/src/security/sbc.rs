use dashmap::DashMap;
use std::net::IpAddr;
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;
use tracing::warn;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpNet {
    ip: IpAddr,
    mask_len: u8,
}

impl IpNet {
    pub fn parse(s: &str) -> Result<Self, String> {
        let parts: Vec<&str> = s.split('/').collect();
        if parts.len() > 2 || parts.iter().any(|part| part.is_empty()) {
            return Err(format!("无效 CIDR 规则: {s}"));
        }
        let ip = IpAddr::from_str(parts[0]).map_err(|e| e.to_string())?;
        let mask_len = if parts.len() > 1 {
            parts[1].parse::<u8>().map_err(|e| e.to_string())?
        } else {
            match ip {
                IpAddr::V4(_) => 32,
                IpAddr::V6(_) => 128,
            }
        };
        let max_mask = if ip.is_ipv4() { 32 } else { 128 };
        if mask_len > max_mask {
            return Err(format!("CIDR 掩码超出范围: {s}"));
        }
        Ok(Self { ip, mask_len })
    }

    #[allow(dead_code)]
    pub fn contains(&self, other: &IpAddr) -> bool {
        match (self.ip, other) {
            (IpAddr::V4(net_v4), IpAddr::V4(other_v4)) => {
                let net_u32 = u32::from_be_bytes(net_v4.octets());
                let other_u32 = u32::from_be_bytes(other_v4.octets());
                let mask = if self.mask_len == 0 {
                    0
                } else if self.mask_len >= 32 {
                    u32::MAX
                } else {
                    u32::MAX << (32 - self.mask_len)
                };
                (net_u32 & mask) == (other_u32 & mask)
            }
            (IpAddr::V6(net_v6), IpAddr::V6(other_v6)) => {
                let net_u128 = u128::from_be_bytes(net_v6.octets());
                let other_u128 = u128::from_be_bytes(other_v6.octets());
                let mask = if self.mask_len == 0 {
                    0
                } else if self.mask_len >= 128 {
                    u128::MAX
                } else {
                    u128::MAX << (128 - self.mask_len)
                };
                (net_u128 & mask) == (other_u128 & mask)
            }
            _ => false,
        }
    }
}

#[derive(Debug)]
pub struct TokenBucket {
    tokens: f64,
    last_update: Instant,
}

impl TokenBucket {
    pub fn new(capacity: f64) -> Self {
        Self {
            tokens: capacity,
            last_update: Instant::now(),
        }
    }

    pub fn take(&mut self, capacity: f64, fill_rate: f64, now: Instant) -> bool {
        let elapsed = now.duration_since(self.last_update).as_secs_f64();
        self.last_update = now;
        self.tokens = (self.tokens + elapsed * fill_rate).min(capacity);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

#[derive(Debug, Default)]
struct IpTrieNode {
    is_match: bool,
    children: [Option<Box<IpTrieNode>>; 2],
}

impl IpTrieNode {
    fn insert(&mut self, ip_bytes: &[u8], mask_len: u8) {
        let mut current = self;
        for bit_idx in 0..mask_len {
            let byte_pos = (bit_idx / 8) as usize;
            let bit_pos = 7 - (bit_idx % 8);
            let bit = ((ip_bytes[byte_pos] >> bit_pos) & 1) as usize;

            if current.is_match {
                return;
            }

            unsafe {
                let next_ptr = &mut **current.children[bit]
                    .get_or_insert_with(|| Box::new(IpTrieNode::default()))
                    as *mut IpTrieNode;
                current = &mut *next_ptr;
            }
        }
        current.is_match = true;
    }

    fn contains(&self, ip_bytes: &[u8]) -> bool {
        let mut current = self;
        if current.is_match {
            return true;
        }

        let max_bits = (ip_bytes.len() * 8) as u8;
        for bit_idx in 0..max_bits {
            let byte_pos = (bit_idx / 8) as usize;
            let bit_pos = 7 - (bit_idx % 8);
            let bit = ((ip_bytes[byte_pos] >> bit_pos) & 1) as usize;

            if let Some(ref next) = current.children[bit] {
                current = next;
                if current.is_match {
                    return true;
                }
            } else {
                break;
            }
        }
        false
    }
}

#[derive(Debug)]
pub struct IpTrie {
    ipv4_root: IpTrieNode,
    ipv6_root: IpTrieNode,
    is_empty: bool,
}

impl Default for IpTrie {
    fn default() -> Self {
        Self {
            ipv4_root: IpTrieNode::default(),
            ipv6_root: IpTrieNode::default(),
            is_empty: true,
        }
    }
}

impl IpTrie {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, net: &IpNet) {
        self.is_empty = false;
        match net.ip {
            IpAddr::V4(v4) => {
                self.ipv4_root.insert(&v4.octets(), net.mask_len);
            }
            IpAddr::V6(v6) => {
                self.ipv6_root.insert(&v6.octets(), net.mask_len);
            }
        }
    }

    pub fn contains(&self, ip: &IpAddr) -> bool {
        match ip {
            IpAddr::V4(v4) => self.ipv4_root.contains(&v4.octets()),
            IpAddr::V6(v6) => self.ipv6_root.contains(&v6.octets()),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.is_empty
    }
}

#[derive(Debug)]
pub struct RateLimiter {
    buckets: DashMap<IpAddr, TokenBucket>,
    capacity: f64,
    fill_rate: f64,
    max_entries: usize,
    insert_count: AtomicUsize,
}

impl RateLimiter {
    pub fn new(capacity: f64, fill_rate: f64) -> Self {
        Self {
            buckets: DashMap::new(),
            capacity,
            fill_rate,
            max_entries: 10_000,
            insert_count: AtomicUsize::new(0),
        }
    }

    pub fn check_rate(&self, ip: IpAddr) -> bool {
        let now = Instant::now();

        let result = {
            let mut bucket = self
                .buckets
                .entry(ip)
                .or_insert_with(|| TokenBucket::new(self.capacity));
            bucket.take(self.capacity, self.fill_rate, now)
        };

        let count = self.insert_count.fetch_add(1, Ordering::Relaxed);
        if count % 1000 == 0 && self.buckets.len() > self.max_entries {
            self.evict_stale(now);
        }

        result
    }

    fn evict_stale(&self, now: Instant) {
        self.buckets
            .retain(|_, b| now.duration_since(b.last_update).as_secs() < 300);
    }
}

#[derive(Debug)]
pub struct SbcEngine {
    allowlist: std::sync::RwLock<IpTrie>,
    blocklist: std::sync::RwLock<IpTrie>,
    rate_limiter: RateLimiter,
    auth_failures: DashMap<IpAddr, (u32, Instant)>,
    locked_ips: DashMap<IpAddr, Instant>,
}

impl SbcEngine {
    pub fn new(allow_rules: &[&str], block_rules: &[&str], capacity: f64, fill_rate: f64) -> Self {
        let mut allowlist = IpTrie::new();
        for rule in allow_rules {
            match IpNet::parse(rule) {
                Ok(net) => allowlist.insert(&net),
                Err(error) => {
                    warn!(rule, %error, "忽略无效 SBC allowlist CIDR 规则");
                }
            }
        }
        let mut blocklist = IpTrie::new();
        for rule in block_rules {
            match IpNet::parse(rule) {
                Ok(net) => blocklist.insert(&net),
                Err(error) => {
                    warn!(rule, %error, "忽略无效 SBC blocklist CIDR 规则");
                }
            }
        }
        Self {
            allowlist: std::sync::RwLock::new(allowlist),
            blocklist: std::sync::RwLock::new(blocklist),
            rate_limiter: RateLimiter::new(capacity, fill_rate),
            auth_failures: DashMap::new(),
            locked_ips: DashMap::new(),
        }
    }

    pub fn is_allowed(&self, ip: IpAddr) -> bool {
        // 1. 检查动态爆破锁定
        if let Some(lock_time) = self.locked_ips.get(&ip) {
            if Instant::now() < *lock_time {
                return false;
            } else {
                drop(lock_time);
                self.locked_ips.remove(&ip);
            }
        }

        // 2. 检查黑名单
        if let Ok(blocklist) = self.blocklist.read() {
            if blocklist.contains(&ip) {
                return false;
            }
        }

        // 3. 检查白名单
        if let Ok(allowlist) = self.allowlist.read() {
            if !allowlist.is_empty() && !allowlist.contains(&ip) {
                return false;
            }
        }

        true
    }

    pub fn register_auth_failure(&self, ip: IpAddr) {
        let now = Instant::now();
        let mut fail_count = 1;

        self.auth_failures
            .entry(ip)
            .and_modify(|(count, last_time)| {
                // 如果上一次失败在 60 秒内，则累加计数
                if now.duration_since(*last_time).as_secs() < 60 {
                    *count += 1;
                } else {
                    // 超过 60 秒则重置计数
                    *count = 1;
                }
                *last_time = now;
                fail_count = *count;
            })
            .or_insert((1, now));

        if fail_count >= 5 {
            let lock_until = now + std::time::Duration::from_secs(600);
            warn!(%ip, "SBC 防暴力破解机制：当前 IP 鉴权失败达到上限，执行动态封禁 10 分钟");
            self.locked_ips.insert(ip, lock_until);
        }
    }

    pub fn update_rules(&self, allow_rules: &[&str], block_rules: &[&str]) {
        let mut allowlist = IpTrie::new();
        for rule in allow_rules {
            match IpNet::parse(rule) {
                Ok(net) => allowlist.insert(&net),
                Err(error) => {
                    warn!(rule, %error, "忽略无效 SBC allowlist CIDR 规则");
                }
            }
        }
        let mut blocklist = IpTrie::new();
        for rule in block_rules {
            match IpNet::parse(rule) {
                Ok(net) => blocklist.insert(&net),
                Err(error) => {
                    warn!(rule, %error, "忽略无效 SBC blocklist CIDR 规则");
                }
            }
        }

        if let Ok(mut w) = self.allowlist.write() {
            *w = allowlist;
        }
        if let Ok(mut w) = self.blocklist.write() {
            *w = blocklist;
        }
    }

    pub fn check_rate(&self, ip: IpAddr) -> bool {
        self.rate_limiter.check_rate(ip)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ip_net_parsing_and_matching() {
        let net = IpNet::parse("192.168.1.0/24").unwrap();
        assert!(net.contains(&IpAddr::from_str("192.168.1.50").unwrap()));
        assert!(!net.contains(&IpAddr::from_str("192.168.2.1").unwrap()));

        let net_single = IpNet::parse("10.0.0.1").unwrap();
        assert!(net_single.contains(&IpAddr::from_str("10.0.0.1").unwrap()));
        assert!(!net_single.contains(&IpAddr::from_str("10.0.0.2").unwrap()));
    }

    #[test]
    fn test_ip_net_rejects_invalid_masks_and_segments() {
        assert!(IpNet::parse("192.168.1.1/33").is_err());
        assert!(IpNet::parse("2001:db8::1/129").is_err());
        assert!(IpNet::parse("192.168.1.1/24/1").is_err());
    }

    #[test]
    fn test_sbc_engine_acl() {
        let sbc = SbcEngine::new(&["192.168.1.0/24"], &["192.168.1.100"], 10.0, 1.0);
        assert!(sbc.is_allowed(IpAddr::from_str("192.168.1.50").unwrap()));
        assert!(!sbc.is_allowed(IpAddr::from_str("192.168.1.100").unwrap())); // blocklisted
        assert!(!sbc.is_allowed(IpAddr::from_str("10.0.0.1").unwrap())); // not in allowlist
    }
}
