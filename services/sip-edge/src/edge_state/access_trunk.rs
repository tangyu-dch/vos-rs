use crate::edge_state::EdgeState;
use crate::sbc;
use std::net::SocketAddr;

#[derive(Debug, Clone)]
pub(crate) struct AccessIpRule {
    pub(crate) trunk_id: String,
    pub(crate) network: sbc::IpNet,
    pub(crate) source_port: Option<u16>,
    pub(crate) transport: String,
}

impl EdgeState {
    pub(crate) fn access_trunk_auth_mode(&self, trunk_id: &str) -> String {
        self.access_trunk_auth_modes
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .get(trunk_id)
            .cloned()
            .unwrap_or_else(|| "none".to_string())
    }

    pub(crate) fn resolve_access_username_to_trunk(&self, username: &str) -> Option<String> {
        self.access_username_to_trunk_id
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .get(username)
            .cloned()
    }

    pub(crate) fn resolve_trunk_billing_account(&self, trunk_id: &str) -> Option<String> {
        self.trunk_billing_accounts
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .get(trunk_id)
            .cloned()
    }

    pub(crate) fn replace_access_sources(
        &self,
        rules: Vec<AccessIpRule>,
        registered_users: Vec<String>,
    ) {
        *self
            .access_ip_rules
            .write()
            .unwrap_or_else(|error| error.into_inner()) = rules;
        *self
            .registered_access_users
            .write()
            .unwrap_or_else(|error| error.into_inner()) = registered_users;
    }

    /// Returns a unique IP-authenticated access trunk. Overlap is rejected at runtime too.
    pub(crate) fn identify_access_trunk(
        &self,
        peer: SocketAddr,
        transport: &str,
    ) -> Result<Option<String>, ()> {
        let matches = self
            .access_ip_rules
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .filter(|rule| rule.transport.eq_ignore_ascii_case(transport))
            .filter(|rule| rule.source_port.is_none_or(|port| port == peer.port()))
            .filter(|rule| rule.network.contains(&peer.ip()))
            .map(|rule| rule.trunk_id.clone())
            .collect::<std::collections::HashSet<_>>();
        match matches.len() {
            0 => Ok(None),
            1 => Ok(matches.into_iter().next()),
            _ => Err(()),
        }
    }

    pub(crate) fn is_registered_access_username(&self, username: &str) -> bool {
        self.registered_access_users
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .any(|configured| configured == username)
    }
}
