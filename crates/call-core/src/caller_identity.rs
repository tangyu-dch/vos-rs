use crate::{CallError, CallResult, GatewayId, SelectedRoute};
use std::collections::HashMap;

/// A resolved public caller number and the only gateway allowed to present it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallerIdentity {
    pub original_number: String,
    pub presented_number: String,
    pub owner_gateway_id: GatewayId,
    pub mode: CallerIdentityMode,
    /// Maximum simultaneous calls allowed to present this number; zero is unlimited.
    pub max_concurrent: u32,
}

/// Caller-number handling applied before an outbound INVITE is built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallerIdentityMode {
    StrictPassthrough,
    Fixed,
    Random,
}

/// Immutable lookup data refreshed alongside the runtime route table.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CallerNumberDirectory {
    owners: HashMap<String, GatewayId>,
    capacities: HashMap<String, u32>,
    numbers_by_gateway: HashMap<GatewayId, Vec<String>>,
}

impl CallerNumberDirectory {
    pub fn owns_number(&self, number: &str, gateway_id: &str) -> bool {
        self.owners
            .get(number)
            .is_some_and(|gid| gid.as_str() == gateway_id)
    }

    pub fn new(entries: impl IntoIterator<Item = (String, String)>) -> Self {
        Self::new_with_capacity(
            entries
                .into_iter()
                .map(|(number, gateway)| (number, gateway, 0)),
        )
    }

    pub fn new_with_capacity(entries: impl IntoIterator<Item = (String, String, u32)>) -> Self {
        let mut directory = Self::default();
        for (number, gateway_id, max_concurrent) in entries {
            let number = number.trim().to_string();
            let gateway_id = gateway_id.trim().to_string();
            if number.is_empty() || gateway_id.is_empty() || !valid_number(&number) {
                continue;
            }
            let gateway_id = GatewayId::new(gateway_id);
            directory.owners.insert(number.clone(), gateway_id.clone());
            directory.capacities.insert(number.clone(), max_concurrent);
            directory
                .numbers_by_gateway
                .entry(gateway_id)
                .or_default()
                .push(number);
        }
        for numbers in directory.numbers_by_gateway.values_mut() {
            numbers.sort();
            numbers.dedup();
        }
        directory
    }

    pub fn resolve(
        &self,
        mode: Option<&str>,
        configured_number: Option<&str>,
        original_number: &str,
        candidates: &[SelectedRoute],
        selection_key: &str,
    ) -> CallResult<Option<CallerIdentity>> {
        let Some(mode) = mode.map(str::trim).filter(|mode| !mode.is_empty()) else {
            return Ok(None);
        };
        match mode {
            // Historical gateways default to `passthrough` and may not have number inventory yet.
            // Strict ownership is enabled only by the new source-policy enum.
            "passthrough" => Ok(None),
            "strict_passthrough" => self.resolve_owned(
                original_number,
                original_number,
                CallerIdentityMode::StrictPassthrough,
                candidates,
            ),
            "fixed_number" | "virtual" | "fixed" => {
                let number = configured_number
                    .map(str::trim)
                    .filter(|number| !number.is_empty())
                    .ok_or_else(|| {
                        CallError::CallerIdentityUnavailable(
                            "fixed caller number is not configured".to_string(),
                        )
                    })?;
                self.resolve_owned(
                    original_number,
                    number,
                    CallerIdentityMode::Fixed,
                    candidates,
                )
            }
            "virtual_pool" | "random" => {
                self.resolve_random(original_number, candidates, selection_key)
            }
            other => Err(CallError::CallerIdentityUnavailable(format!(
                "unsupported caller identity mode: {other}"
            ))),
        }
    }

    fn resolve_owned(
        &self,
        original_number: &str,
        presented_number: &str,
        mode: CallerIdentityMode,
        candidates: &[SelectedRoute],
    ) -> CallResult<Option<CallerIdentity>> {
        if !valid_number(presented_number) {
            return Err(CallError::CallerIdentityUnavailable(
                "caller number contains unsupported characters".to_string(),
            ));
        }
        let owner_gateway_id = self.owners.get(presented_number).cloned().ok_or_else(|| {
            CallError::CallerIdentityUnavailable(format!(
                "caller number {presented_number} has no enabled owner gateway"
            ))
        })?;
        ensure_gateway_is_candidate(&owner_gateway_id, candidates)?;
        Ok(Some(CallerIdentity {
            original_number: original_number.to_string(),
            presented_number: presented_number.to_string(),
            owner_gateway_id,
            mode,
            max_concurrent: self.capacities.get(presented_number).copied().unwrap_or(0),
        }))
    }

    fn resolve_random(
        &self,
        original_number: &str,
        candidates: &[SelectedRoute],
        selection_key: &str,
    ) -> CallResult<Option<CallerIdentity>> {
        let available = candidates
            .iter()
            .filter_map(|candidate| {
                self.numbers_by_gateway
                    .get(&candidate.target.gateway_id)
                    .map(|numbers| (&candidate.target.gateway_id, numbers))
            })
            .flat_map(|(gateway, numbers)| numbers.iter().map(move |number| (gateway, number)))
            .collect::<Vec<_>>();
        if available.is_empty() {
            return Err(CallError::CallerIdentityUnavailable(
                "no enabled caller number is available for the selected gateways".to_string(),
            ));
        }
        let index = stable_selection_index(selection_key, available.len());
        let (owner_gateway_id, presented_number) = available[index];
        Ok(Some(CallerIdentity {
            original_number: original_number.to_string(),
            presented_number: presented_number.clone(),
            owner_gateway_id: owner_gateway_id.clone(),
            mode: CallerIdentityMode::Random,
            max_concurrent: self.capacities.get(presented_number).copied().unwrap_or(0),
        }))
    }
}

fn ensure_gateway_is_candidate(
    gateway_id: &GatewayId,
    candidates: &[SelectedRoute],
) -> CallResult<()> {
    if candidates
        .iter()
        .any(|candidate| candidate.target.gateway_id == *gateway_id)
    {
        Ok(())
    } else {
        Err(CallError::CallerIdentityUnavailable(format!(
            "caller number owner gateway {} is outside the allowed termination routes",
            gateway_id.as_str()
        )))
    }
}

fn valid_number(number: &str) -> bool {
    let digits = number.strip_prefix('+').unwrap_or(number);
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
}

fn stable_selection_index(key: &str, len: usize) -> usize {
    let hash = key.bytes().fold(0xcbf29ce484222325_u64, |hash, byte| {
        hash.wrapping_mul(0x100000001b3) ^ u64::from(byte)
    });
    (hash as usize) % len
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RouteTarget;
    use sip_core::SipUri;
    use std::str::FromStr;

    fn candidate(gateway: &str) -> SelectedRoute {
        SelectedRoute {
            route_id: "route".to_string(),
            target: RouteTarget::new(gateway, "example.com", Some(5060)),
            outbound_uri: SipUri::from_str("sip:callee@example.com").expect("valid URI"),
        }
    }

    #[test]
    fn strict_passthrough_requires_an_owned_number() {
        let directory =
            CallerNumberDirectory::new_with_capacity([("13800138000".into(), "gw1".into(), 7)]);
        let resolved = directory
            .resolve(
                Some("strict_passthrough"),
                None,
                "13800138000",
                &[candidate("gw1")],
                "call-1",
            )
            .expect("owned caller should resolve")
            .expect("policy should create identity");
        assert_eq!(resolved.presented_number, "13800138000");
        assert_eq!(resolved.owner_gateway_id.as_str(), "gw1");
        assert_eq!(resolved.max_concurrent, 7);

        assert!(directory
            .resolve(
                Some("strict_passthrough"),
                None,
                "13900139000",
                &[candidate("gw1")],
                "call-2",
            )
            .is_err());
    }

    #[test]
    fn legacy_passthrough_remains_compatible_without_number_inventory() {
        let directory = CallerNumberDirectory::default();
        assert_eq!(
            directory
                .resolve(
                    Some("passthrough"),
                    None,
                    "1001",
                    &[candidate("gw1")],
                    "legacy-call",
                )
                .expect("legacy passthrough must remain available"),
            None
        );
    }

    #[test]
    fn fixed_number_cannot_escape_the_allowed_gateway_candidates() {
        let directory = CallerNumberDirectory::new([("13800138000".into(), "gw2".into())]);
        assert!(directory
            .resolve(
                Some("virtual"),
                Some("13800138000"),
                "1001",
                &[candidate("gw1")],
                "call-1",
            )
            .is_err());
    }

    #[test]
    fn random_selection_returns_number_and_owner_as_one_value() {
        let directory = CallerNumberDirectory::new([
            ("13800138000".into(), "gw1".into()),
            ("13900139000".into(), "gw2".into()),
        ]);
        let resolved = directory
            .resolve(
                Some("random"),
                None,
                "1001",
                &[candidate("gw1"), candidate("gw2")],
                "stable-call",
            )
            .expect("pool should resolve")
            .expect("policy should create identity");
        assert!(
            resolved.presented_number == "13800138000"
                || resolved.presented_number == "13900139000"
        );
        assert_eq!(
            directory.owners[&resolved.presented_number],
            resolved.owner_gateway_id
        );
    }
}
