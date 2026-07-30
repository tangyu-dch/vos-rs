use crate::edge_state::EdgeState;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};

#[derive(Debug, Default)]
pub(crate) struct GatewayIdentityCache {
    exact_endpoints: HashMap<SocketAddr, Option<String>>,
    unique_ips: HashMap<IpAddr, Option<String>>,
    trunk_targets: HashMap<String, String>,
}

impl GatewayIdentityCache {
    fn replace(&mut self, gateways: impl IntoIterator<Item = (String, u16, String)>) {
        self.exact_endpoints.clear();
        self.unique_ips.clear();
        self.trunk_targets.clear();
        for (host, port, trunk_id) in gateways {
            self.trunk_targets
                .insert(trunk_id.clone(), format_gateway_target(&host, port));
            let Some(ip) = parse_normalized_ip(&host) else {
                continue;
            };
            merge_gateway_identity(
                &mut self.exact_endpoints,
                SocketAddr::new(ip, port),
                &trunk_id,
            );
            merge_gateway_identity(&mut self.unique_ips, ip, &trunk_id);
        }
    }

    fn identify(&self, peer: SocketAddr) -> Option<String> {
        let peer = normalize_socket_addr(peer);
        self.exact_endpoints
            .get(&peer)
            .cloned()
            .flatten()
            .or_else(|| self.unique_ips.get(&peer.ip()).cloned().flatten())
    }
}

fn format_gateway_target(host: &str, port: u16) -> String {
    let host = host.trim();
    if host.starts_with('[') || !host.contains(':') {
        format!("{host}:{port}")
    } else {
        format!("[{host}]:{port}")
    }
}

fn merge_gateway_identity<K>(identities: &mut HashMap<K, Option<String>>, key: K, trunk_id: &str)
where
    K: std::hash::Hash + Eq,
{
    identities
        .entry(key)
        .and_modify(|current| {
            if current.as_deref() != Some(trunk_id) {
                *current = None;
            }
        })
        .or_insert_with(|| Some(trunk_id.to_string()));
}

fn parse_normalized_ip(host: &str) -> Option<IpAddr> {
    let host = host.trim();
    let host = host
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(host);
    host.parse().ok().map(normalize_ip)
}

fn normalize_socket_addr(address: SocketAddr) -> SocketAddr {
    SocketAddr::new(normalize_ip(address.ip()), address.port())
}

fn normalize_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(ipv6) => ipv6
            .to_ipv4_mapped()
            .map(IpAddr::V4)
            .unwrap_or(IpAddr::V6(ipv6)),
        ipv4 => ipv4,
    }
}

impl EdgeState {
    pub(crate) async fn identify_egress_trunk(&self, peer: SocketAddr) -> Option<String> {
        #[cfg(test)]
        {
            let peer_ip = normalize_ip(peer.ip()).to_string();
            if self
                .test_gateways
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .contains(&peer_ip)
            {
                return Some("test-gateway".to_string());
            }
        }
        self.gateway_cache
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .identify(peer)
    }

    /// Replaces the egress identity cache with configured SIP endpoints.
    pub(crate) fn replace_gateway_endpoint_cache(
        &self,
        gateways: impl IntoIterator<Item = (String, u16, String)>,
    ) {
        let mut cache = self
            .gateway_cache
            .write()
            .unwrap_or_else(|error| error.into_inner());
        cache.replace(gateways);
    }

    pub(crate) fn gateway_target(&self, trunk_id: &str) -> Option<String> {
        self.gateway_cache
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .trunk_targets
            .get(trunk_id)
            .cloned()
    }

    #[cfg(test)]
    pub(crate) fn replace_gateway_cache(
        &self,
        gateways: impl IntoIterator<Item = (String, String)>,
    ) {
        self.replace_gateway_endpoint_cache(
            gateways
                .into_iter()
                .map(|(host, trunk_id)| (host, 5060, trunk_id)),
        );
    }
}

#[cfg(test)]
mod gateway_identity_tests {
    use super::GatewayIdentityCache;
    use crate::edge_state::build_balance_check;

    fn identify(cache: &GatewayIdentityCache, peer: &str) -> Option<String> {
        cache.identify(peer.parse().expect("valid test peer"))
    }

    #[test]
    fn exact_endpoint_distinguishes_trunks_on_the_same_ip() {
        let mut cache = GatewayIdentityCache::default();
        cache.replace([
            ("127.0.0.1".to_string(), 5170, "carrier-a".to_string()),
            ("127.0.0.1".to_string(), 5171, "carrier-b".to_string()),
        ]);

        assert_eq!(
            identify(&cache, "127.0.0.1:5170").as_deref(),
            Some("carrier-a")
        );
        assert_eq!(
            identify(&cache, "127.0.0.1:5171").as_deref(),
            Some("carrier-b")
        );
        assert_eq!(identify(&cache, "127.0.0.1:5199"), None);
    }

    #[test]
    fn ip_fallback_only_accepts_a_unique_trunk() {
        let mut cache = GatewayIdentityCache::default();
        cache.replace([
            ("192.0.2.10".to_string(), 5060, "carrier-a".to_string()),
            ("192.0.2.10".to_string(), 5070, "carrier-a".to_string()),
        ]);
        assert_eq!(
            identify(&cache, "192.0.2.10:5090").as_deref(),
            Some("carrier-a")
        );

        cache.replace([
            ("192.0.2.10".to_string(), 5060, "carrier-a".to_string()),
            ("192.0.2.10".to_string(), 5070, "carrier-b".to_string()),
        ]);
        assert_eq!(identify(&cache, "192.0.2.10:5090"), None);
    }

    #[test]
    fn ipv4_mapped_ipv6_peers_match_ipv4_configuration() {
        let mut cache = GatewayIdentityCache::default();
        cache.replace([("192.0.2.10".to_string(), 5060, "carrier-a".to_string())]);

        assert_eq!(
            identify(&cache, "[::ffff:192.0.2.10]:5060").as_deref(),
            Some("carrier-a")
        );
    }

    #[test]
    fn duplicate_endpoint_owned_by_multiple_trunks_is_ambiguous() {
        let mut cache = GatewayIdentityCache::default();
        cache.replace([
            ("192.0.2.10".to_string(), 5060, "carrier-a".to_string()),
            ("192.0.2.10".to_string(), 5060, "carrier-b".to_string()),
        ]);

        assert_eq!(identify(&cache, "192.0.2.10:5060"), None);
    }

    #[test]
    fn billing_check_distinguishes_missing_data_and_uses_credit_limit() {
        let missing_account = build_balance_check(None, None, Some((60, 0.5)));
        assert!(!missing_account.account_found);
        assert!(!missing_account.has_balance);

        let missing_rate = build_balance_check(Some(10.0), Some(0.0), None);
        assert!(!missing_rate.rate_found);
        assert!(!missing_rate.has_balance);

        let credit = build_balance_check(Some(-0.25), Some(1.0), Some((6, 0.05)));
        assert!(credit.has_balance);
        assert_eq!(credit.balance + credit.credit_limit, 0.75);
    }
}
