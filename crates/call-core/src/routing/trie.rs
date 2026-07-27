use super::types::Route;
use std::collections::HashMap;

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct PrefixTrieNode {
    routes: Vec<Route>,
    children: HashMap<char, PrefixTrieNode>,
}

impl PrefixTrieNode {
    pub(crate) fn insert(&mut self, prefix: &str, route: Route) {
        let mut current = self;
        for c in prefix.chars() {
            current = current.children.entry(c).or_default();
        }
        current.routes.push(route);
    }

    pub(crate) fn query(&self, destination: &str, out: &mut Vec<Route>) {
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
