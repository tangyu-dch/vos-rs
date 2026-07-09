use dashmap::DashMap;
use std::net::IpAddr;
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpNet {
    ip: IpAddr,
    mask_len: u8,
}

impl IpNet {
    pub fn parse(s: &str) -> Result<Self, String> {
        let parts: Vec<&str> = s.split('/').collect();
        let ip = IpAddr::from_str(parts[0]).map_err(|e| e.to_string())?;
        let mask_len = if parts.len() > 1 {
            parts[1].parse::<u8>().map_err(|e| e.to_string())?
        } else {
            match ip {
                IpAddr::V4(_) => 32,
                IpAddr::V6(_) => 128,
            }
        };
        Ok(Self { ip, mask_len })
    }

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
    allowlist: Vec<IpNet>,
    blocklist: Vec<IpNet>,
    rate_limiter: RateLimiter,
}

impl SbcEngine {
    pub fn new(allow_rules: &[&str], block_rules: &[&str], capacity: f64, fill_rate: f64) -> Self {
        let allowlist = allow_rules
            .iter()
            .filter_map(|r| IpNet::parse(r).ok())
            .collect();
        let blocklist = block_rules
            .iter()
            .filter_map(|r| IpNet::parse(r).ok())
            .collect();
        Self {
            allowlist,
            blocklist,
            rate_limiter: RateLimiter::new(capacity, fill_rate),
        }
    }

    pub fn is_allowed(&self, ip: IpAddr) -> bool {
        for net in &self.blocklist {
            if net.contains(&ip) {
                return false;
            }
        }
        if !self.allowlist.is_empty() {
            let mut found = false;
            for net in &self.allowlist {
                if net.contains(&ip) {
                    found = true;
                    break;
                }
            }
            if !found {
                return false;
            }
        }
        true
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
    fn test_sbc_engine_acl() {
        let sbc = SbcEngine::new(&["192.168.1.0/24"], &["192.168.1.100"], 10.0, 1.0);
        assert!(sbc.is_allowed(IpAddr::from_str("192.168.1.50").unwrap()));
        assert!(!sbc.is_allowed(IpAddr::from_str("192.168.1.100").unwrap())); // blocklisted
        assert!(!sbc.is_allowed(IpAddr::from_str("10.0.0.1").unwrap())); // not in allowlist
    }
}
