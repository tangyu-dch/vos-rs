use super::health::GatewayHealthTracker;
use super::types::{Route, SelectedRoute};
use crate::{CallError, CallResult};
use sip_core::SipUri;

#[derive(Debug, Clone, Default, PartialEq)]
struct PrefixTrieNode {
    routes: Vec<Route>,
    children: std::collections::HashMap<char, PrefixTrieNode>,
}

impl PrefixTrieNode {
    fn insert(&mut self, prefix: &str, route: Route) {
        let mut current = self;
        for c in prefix.chars() {
            current = current.children.entry(c).or_default();
        }
        current.routes.push(route);
    }

    fn query(&self, destination: &str, out: &mut Vec<Route>) {
        let mut current = self;
        for route in &current.routes {
            out.push(route.clone());
        }
        for c in destination.chars() {
            if let Some(next) = current.children.get(&c) {
                current = next;
                for route in &current.routes {
                    out.push(route.clone());
                }
            } else {
                break;
            }
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RouteTable {
    routes: Vec<Route>,
    trie: PrefixTrieNode,
}

fn weighted_shuffle(mut items: Vec<&Route>) -> Vec<&Route> {
    use rand::Rng;
    let mut result = Vec::with_capacity(items.len());
    let mut rng = rand::thread_rng();

    while !items.is_empty() {
        let total_weight: u32 = items.iter().map(|item| item.weight.max(1)).sum();
        if total_weight == 0 {
            result.extend(items);
            break;
        }
        let mut target = rng.gen_range(0..total_weight);
        let mut chosen_idx = 0;
        for (idx, item) in items.iter().enumerate() {
            let w = item.weight.max(1);
            if target < w {
                chosen_idx = idx;
                break;
            }
            target -= w;
        }
        result.push(items.remove(chosen_idx));
    }
    result
}

impl RouteTable {
    pub fn new(routes: Vec<Route>) -> Self {
        let mut trie = PrefixTrieNode::default();
        for route in &routes {
            trie.insert(&route.prefix, route.clone());
        }
        Self { routes, trie }
    }

    pub fn clear(&mut self) {
        self.routes.clear();
        self.trie = PrefixTrieNode::default();
    }

    pub fn add_route(&mut self, route: Route) {
        self.trie.insert(&route.prefix, route.clone());
        self.routes.push(route);
    }

    pub fn select(&self, destination_uri: &SipUri) -> CallResult<SelectedRoute> {
        let candidates = self.select_candidates(destination_uri)?;
        candidates.first().cloned().ok_or_else(|| {
            CallError::NoRouteForDestination(
                destination_uri
                    .user
                    .as_deref()
                    .unwrap_or_default()
                    .to_string(),
            )
        })
    }

    pub fn select_candidates(&self, destination_uri: &SipUri) -> CallResult<Vec<SelectedRoute>> {
        let destination = destination_uri
            .user
            .as_deref()
            .ok_or(CallError::InvalidDestinationUri)?;

        let mut matched_buffer = Vec::new();
        self.trie.query(destination, &mut matched_buffer);

        if matched_buffer.is_empty() {
            return Err(CallError::NoRouteForDestination(destination.to_string()));
        }

        let mut matching_routes: Vec<&Route> = matched_buffer.iter().collect();

        matching_routes.sort_by(|left, right| {
            right
                .prefix
                .len()
                .cmp(&left.prefix.len())
                .then_with(|| right.priority.cmp(&left.priority))
                .then_with(|| {
                    left.cost
                        .partial_cmp(&right.cost)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        });

        let mut grouped_routes = Vec::new();
        let mut current_group = Vec::new();

        for route in matching_routes {
            if current_group.is_empty() {
                current_group.push(route);
            } else {
                let first = current_group[0];
                let is_equivalent = first.prefix.len() == route.prefix.len()
                    && first.priority == route.priority
                    && (first.cost - route.cost).abs() < 1e-9;
                if is_equivalent {
                    current_group.push(route);
                } else {
                    grouped_routes.push(weighted_shuffle(current_group));
                    current_group = vec![route];
                }
            }
        }
        if !current_group.is_empty() {
            grouped_routes.push(weighted_shuffle(current_group));
        }

        let final_routes: Vec<&Route> = grouped_routes.into_iter().flatten().collect();

        let mut candidates = Vec::with_capacity(final_routes.len());
        for route in final_routes {
            candidates.push(SelectedRoute {
                route_id: route.id.clone(),
                target: route.target.clone(),
                outbound_uri: route.target.outbound_uri_for(destination_uri)?,
            });
        }

        Ok(candidates)
    }

    pub fn select_candidates_for_direction(
        &self,
        destination_uri: &SipUri,
        call_direction: &str,
    ) -> CallResult<Vec<SelectedRoute>> {
        let destination = destination_uri
            .user
            .as_deref()
            .ok_or(CallError::InvalidDestinationUri)?;

        let mut matched_buffer = Vec::new();
        self.trie.query(destination, &mut matched_buffer);

        let mut matching_routes: Vec<&Route> = matched_buffer
            .iter()
            .filter(|route| route.target.has_capacity(call_direction))
            .collect();

        if matching_routes.is_empty() {
            return Err(CallError::NoRouteForDestination(destination.to_string()));
        }

        matching_routes.sort_by(|left, right| {
            right
                .prefix
                .len()
                .cmp(&left.prefix.len())
                .then_with(|| right.priority.cmp(&left.priority))
                .then_with(|| {
                    left.cost
                        .partial_cmp(&right.cost)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        });

        let mut grouped_routes = Vec::new();
        let mut current_group = Vec::new();

        for route in matching_routes {
            if current_group.is_empty() {
                current_group.push(route);
            } else {
                let first = current_group[0];
                let is_equivalent = first.prefix.len() == route.prefix.len()
                    && first.priority == route.priority
                    && (first.cost - route.cost).abs() < 1e-9;
                if is_equivalent {
                    current_group.push(route);
                } else {
                    grouped_routes.push(weighted_shuffle(current_group));
                    current_group = vec![route];
                }
            }
        }
        if !current_group.is_empty() {
            grouped_routes.push(weighted_shuffle(current_group));
        }

        let final_routes: Vec<&Route> = grouped_routes.into_iter().flatten().collect();

        let mut candidates = Vec::with_capacity(final_routes.len());
        for route in final_routes {
            candidates.push(SelectedRoute {
                route_id: route.id.clone(),
                target: route.target.clone(),
                outbound_uri: route.target.outbound_uri_for(destination_uri)?,
            });
        }

        Ok(candidates)
    }

    pub fn select_healthy_candidates(
        &self,
        destination_uri: &SipUri,
        health: &GatewayHealthTracker,
        call_direction: Option<&str>,
    ) -> CallResult<Vec<SelectedRoute>> {
        let all_candidates = if let Some(dir) = call_direction {
            self.select_candidates_for_direction(destination_uri, dir)?
        } else {
            self.select_candidates(destination_uri)?
        };

        let available: Vec<SelectedRoute> = all_candidates
            .iter()
            .filter(|c| {
                let gid = c.target.gateway_id.as_str();
                health.has_capacity(gid, c.target.max_capacity) && health.is_available(gid)
            })
            .cloned()
            .collect();

        if available.is_empty() {
            warn_all_gateways_unhealthy(&all_candidates);
            return Err(CallError::GatewayUnavailable(
                destination_uri
                    .user
                    .as_deref()
                    .unwrap_or_default()
                    .to_string(),
            ));
        }

        let first_gid = available[0].target.gateway_id.as_str();
        if !health.try_acquire(first_gid) {
            for c in available.iter().skip(1) {
                let gid = c.target.gateway_id.as_str();
                if health.try_acquire(gid) {
                    let mut result: Vec<SelectedRoute> = Vec::with_capacity(available.len());
                    result.push(c.clone());
                    for other in available.iter() {
                        if other.target.gateway_id.as_str() != gid {
                            result.push(other.clone());
                        }
                    }
                    return Ok(result);
                }
            }
            return Err(CallError::GatewayUnavailable(
                destination_uri
                    .user
                    .as_deref()
                    .unwrap_or_default()
                    .to_string(),
            ));
        }

        Ok(available)
    }

    pub fn routes(&self) -> &[Route] {
        &self.routes
    }

    pub fn is_empty(&self) -> bool {
        self.routes.is_empty()
    }
}

fn warn_all_gateways_unhealthy(candidates: &[SelectedRoute]) {
    let _ = candidates;
}
