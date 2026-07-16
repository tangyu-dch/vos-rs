use std::{net::IpAddr, time::Instant};

use dashmap::DashMap;

use crate::{config::RouterConfig, metrics};

#[derive(Debug, Clone)]
struct IpNetwork {
    network: u128,
    mask: u128,
    ipv6: bool,
}

impl IpNetwork {
    fn parse(value: &str) -> Result<Self, &'static str> {
        let (address, prefix) = value.split_once('/').ok_or("CIDR 缺少掩码")?;
        let address: IpAddr = address.parse().map_err(|_| "CIDR 地址无效")?;
        let prefix: u32 = prefix.parse().map_err(|_| "CIDR 掩码无效")?;
        let (raw, bits, ipv6) = match address {
            IpAddr::V4(address) => (u32::from(address) as u128, 32, false),
            IpAddr::V6(address) => (u128::from(address), 128, true),
        };
        if prefix > bits {
            return Err("CIDR 掩码超出地址长度");
        }
        let mask = if prefix == 0 {
            0
        } else {
            u128::MAX << (bits - prefix)
        };
        let mask = if ipv6 { mask } else { mask & u32::MAX as u128 };
        Ok(Self {
            network: raw & mask,
            mask,
            ipv6,
        })
    }

    fn contains(&self, address: IpAddr) -> bool {
        let (raw, ipv6) = match address {
            IpAddr::V4(address) => (u32::from(address) as u128, false),
            IpAddr::V6(address) => (u128::from(address), true),
        };
        self.ipv6 == ipv6 && raw & self.mask == self.network
    }
}

#[derive(Debug)]
struct TokenBucket {
    tokens: f64,
    updated_at: Instant,
}

/// 原生 SIP 入口 ACL 与每 IP 令牌桶。
pub(crate) struct RouterGuard {
    allow: Vec<IpNetwork>,
    block: Vec<IpNetwork>,
    buckets: DashMap<IpAddr, TokenBucket>,
    capacity: f64,
    fill_rate: f64,
    max_entries: usize,
}

impl RouterGuard {
    pub(crate) fn from_config(config: &RouterConfig) -> Result<Self, &'static str> {
        Ok(Self {
            allow: parse_rules(&config.acl_allow)?,
            block: parse_rules(&config.acl_block)?,
            buckets: DashMap::new(),
            capacity: f64::from(config.rate_limit_capacity),
            fill_rate: f64::from(config.rate_limit_fill_rate),
            max_entries: config.rate_limit_max_entries,
        })
    }

    pub(crate) fn allow(&self, address: IpAddr, trusted_backend: bool) -> bool {
        if trusted_backend {
            return true;
        }
        if self.block.iter().any(|rule| rule.contains(address))
            || (!self.allow.is_empty() && !self.allow.iter().any(|rule| rule.contains(address)))
        {
            metrics::security_rejected();
            return false;
        }
        let now = Instant::now();
        let mut bucket = self.buckets.entry(address).or_insert(TokenBucket {
            tokens: self.capacity,
            updated_at: now,
        });
        let elapsed = now.duration_since(bucket.updated_at).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * self.fill_rate).min(self.capacity);
        bucket.updated_at = now;
        if bucket.tokens < 1.0 {
            metrics::security_rejected();
            return false;
        }
        bucket.tokens -= 1.0;
        drop(bucket);
        if self.buckets.len() > self.max_entries {
            self.buckets
                .retain(|_, bucket| now.duration_since(bucket.updated_at).as_secs() < 60);
        }
        true
    }
}

fn parse_rules(values: &[String]) -> Result<Vec<IpNetwork>, &'static str> {
    values.iter().map(|value| IpNetwork::parse(value)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cidr_rules_support_ipv4_and_ipv6() {
        assert!(IpNetwork::parse("192.0.2.0/24")
            .expect("v4")
            .contains("192.0.2.9".parse().expect("ip")));
        assert!(IpNetwork::parse("2001:db8::/32")
            .expect("v6")
            .contains("2001:db8::1".parse().expect("ip")));
        assert!(IpNetwork::parse("192.0.2.0/33").is_err());
    }

    #[test]
    fn test_token_bucket_rejects_after_capacity() {
        let mut config = test_config();
        config.rate_limit_capacity = 1;
        config.rate_limit_fill_rate = 1;
        let guard = RouterGuard::from_config(&config).expect("guard");
        let address = "192.0.2.10".parse().expect("ip");
        assert!(guard.allow(address, false));
        assert!(!guard.allow(address, false));
        assert!(guard.allow(address, true));
    }

    fn test_config() -> RouterConfig {
        RouterConfig {
            udp_bind: "127.0.0.1:5060".to_string(),
            tcp_bind: "127.0.0.1:5060".to_string(),
            advertised_addr: "127.0.0.1:5060".to_string(),
            redis_url: "redis://127.0.0.1/0".to_string(),
            node_key_prefix: "test".to_string(),
            discovery_interval_secs: 1,
            transaction_ttl_secs: 64,
            dialog_route_ttl_secs: 60,
            udp_workers: 1,
            udp_queue_capacity: 64,
            max_transactions: 1024,
            tcp_max_connections: 64,
            tcp_write_queue_capacity: 64,
            tcp_idle_timeout_secs: 30,
            tcp_connect_timeout_secs: 1,
            manage_bind: "127.0.0.1:8083".to_string(),
            acl_allow: Vec::new(),
            acl_block: Vec::new(),
            rate_limit_capacity: 10,
            rate_limit_fill_rate: 10,
            rate_limit_max_entries: 1024,
        }
    }
}
