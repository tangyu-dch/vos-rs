use crate::edge_state::EdgeState;
use cdr_core::SipFlowRecord;
use sip_core::parse_message;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{channel, Receiver, Sender};
use time::OffsetDateTime;

pub(crate) struct SipFlowWriter {
    rx: Receiver<SipFlowRecord>,
    edge_state: Arc<EdgeState>,
}

impl SipFlowWriter {
    pub(crate) fn start(
        edge_state: Arc<EdgeState>,
        queue_capacity: usize,
    ) -> Sender<SipFlowRecord> {
        let (tx, rx) = channel(queue_capacity);
        let writer = Self { rx, edge_state };
        tokio::spawn(writer.run());
        tx
    }

    async fn run(mut self) {
        let db = match &self.edge_state.db_store {
            Some(db) => db.clone(),
            None => {
                tracing::info!("SipFlowWriter: no DB store configured, exiting background writer");
                return;
            }
        };

        let mut batch = Vec::new();
        let mut interval = tokio::time::interval(Duration::from_millis(100));
        let mut cleanup_interval = tokio::time::interval(Duration::from_secs(60));

        loop {
            tokio::select! {
                Some(record) = self.rx.recv() => {
                    batch.push(record);
                    if batch.len() >= 100 {
                        if let Err(e) = db.insert_sip_flows_batch(&batch).await {
                            tracing::error!("failed to insert sip flows batch: {e}");
                        }
                        batch.clear();
                    }
                }
                _ = interval.tick() => {
                    if !batch.is_empty() {
                        if let Err(e) = db.insert_sip_flows_batch(&batch).await {
                            tracing::error!("failed to insert sip flows batch: {e}");
                        }
                        batch.clear();
                    }
                }
                _ = cleanup_interval.tick() => {
                    // 1. Delete expired sip flows from the database
                    let retention_days = self.edge_state.sipflow_retention_days.load(std::sync::atomic::Ordering::Relaxed);
                    if retention_days > 0 {
                        match db.delete_expired_sip_flows(retention_days).await {
                            Ok(rows) if rows > 0 => {
                                tracing::debug!("SipFlowWriter: cleaned up {rows} expired SIP flow traces");
                            }
                            Err(e) => {
                                tracing::error!("failed to delete expired sip flows: {e}");
                            }
                            _ => {}
                        }
                    }

                    // 2. In-memory clean up of old matched call IDs to prevent memory leaks
                    let now = Instant::now();
                    self.edge_state.matched_call_ids.retain(|_, start_time| {
                        now.duration_since(*start_time) < Duration::from_secs(7200) // retain for max 2 hours
                    });
                    
                    // Clean up matching caller addresses
                    let matched_keys: std::collections::HashSet<String> = self.edge_state.matched_call_ids
                        .iter()
                        .map(|r| r.key().clone())
                        .collect();
                    self.edge_state.call_caller_addrs.retain(|k, _| {
                        matched_keys.contains(k)
                    });
                }
            }
        }
    }
}

fn extract_user(val: &str) -> Option<String> {
    if let Some(sip_pos) = val.find("sip:") {
        let rest = &val[sip_pos + 4..];
        let end_pos = rest.find(['@', ';', '>']);
        if let Some(at_pos) = rest.find('@') {
            let user = &rest[..at_pos];
            return Some(user.trim_start_matches('"').trim_end_matches('"').trim().to_string());
        }
        if let Some(ep) = end_pos {
            return Some(rest[..ep].trim().to_string());
        }
        return Some(rest.trim().to_string());
    }
    None
}

impl EdgeState {
    pub(crate) fn capture_sip_packet(&self, bytes: &[u8], direction: &str, peer_addr: SocketAddr) {
        // Parse the message to extract Call-ID and headers
        let Ok(msg) = parse_message(bytes) else {
            return;
        };

        let Some(call_id_val) = msg.headers().get("call-id") else {
            return;
        };
        let call_id = call_id_val.as_str().trim().to_string();

        let msg_method = match &msg {
            sip_core::SipMessageBorrow::Request(req) => req.method.as_str().to_string(),
            sip_core::SipMessageBorrow::Response(resp) => format!("{} {}", resp.status_code, resp.reason_phrase),
        };

        // Determine if we should save caller address
        if direction == "in" {
            if let sip_core::SipMessageBorrow::Request(req) = &msg {
                if req.method == sip_core::Method::Invite {
                    self.call_caller_addrs.entry(call_id.clone()).or_insert(peer_addr);
                }
            }
        }

        // Whitelist check
        let mut whitelisted = false;
        if self.matched_call_ids.contains_key(&call_id) {
            whitelisted = true;
        } else {
            let whitelist = self.sipflow_whitelist.read().unwrap().clone();
            if whitelist == "*" {
                whitelisted = true;
            } else if !whitelist.is_empty() {
                let from_user = msg.headers().get("from").and_then(|h| extract_user(h.as_str()));
                let to_user = msg.headers().get("to").and_then(|h| extract_user(h.as_str()));
                
                let parts: Vec<&str> = whitelist.split(',').map(|s| s.trim()).collect();
                if let Some(fu) = &from_user {
                    if parts.contains(&fu.as_str()) {
                        whitelisted = true;
                    }
                }
                if let Some(tu) = &to_user {
                    if parts.contains(&tu.as_str()) {
                        whitelisted = true;
                    }
                }
            }

            if whitelisted {
                self.matched_call_ids.insert(call_id.clone(), Instant::now());
            }
        }

        if !whitelisted {
            return;
        }

        // Determine direction string for api-server and frontend
        let dir = if direction == "in" {
            let is_caller = self.call_caller_addrs.get(&call_id)
                .map(|addr_guard| *addr_guard == peer_addr)
                .unwrap_or(false);
            if is_caller {
                "uac_to_b2bua".to_string()
            } else {
                "uas_to_b2bua".to_string()
            }
        } else {
            let is_caller = self.call_caller_addrs.get(&call_id)
                .map(|addr_guard| *addr_guard == peer_addr)
                .unwrap_or(false);
            if is_caller {
                "b2bua_to_uac".to_string()
            } else {
                "b2bua_to_uas".to_string()
            }
        };

        // Get local socket address (or config bind if onceLock not initialized)
        let local_addr = self.socket.get()
            .and_then(|s| s.local_addr().ok())
            .map(|a| a.to_string())
            .unwrap_or_else(|| "0.0.0.0:5060".to_string());

        let (from_str, to_str) = if direction == "in" {
            (peer_addr.to_string(), local_addr)
        } else {
            (local_addr, peer_addr.to_string())
        };

        let raw_message = String::from_utf8_lossy(bytes).into_owned();

        let record = SipFlowRecord {
            id: 0, // generated by DB
            call_id,
            method: msg_method,
            direction: dir,
            from_addr: from_str,
            to_addr: to_str,
            raw_message,
            timestamp: OffsetDateTime::now_utc(),
        };

        if let Some(tx) = self.sip_flow_tx.get() {
            let _ = tx.try_send(record.clone());
        }

        // If BYE request or response to BYE seen, we can clean up after a small delay
        let is_bye = match &msg {
            sip_core::SipMessageBorrow::Request(req) => req.method == sip_core::Method::Bye,
            sip_core::SipMessageBorrow::Response(_resp) => {
                msg.headers().get("cseq")
                    .map(|h| h.as_str().to_ascii_uppercase().contains("BYE"))
                    .unwrap_or(false)
            }
        };

        if is_bye {
            let self_weak = self.self_weak.get().cloned();
            let call_id_clone = record.call_id.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(10)).await;
                if let Some(state) = self_weak.and_then(|w| w.upgrade()) {
                    state.matched_call_ids.remove(&call_id_clone);
                    state.call_caller_addrs.remove(&call_id_clone);
                }
            });
        }
    }
}
