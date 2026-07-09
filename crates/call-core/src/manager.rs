use crate::{
    Call, CallCdr, CallError, CallId, CallQualityMetrics, CallResult, CallState, RouteTable,
};
use dashmap::DashMap;
use sip_core::{SipRequest, SipResponse};
use std::sync::{Mutex, RwLock};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboundInviteOutcome {
    pub call_id: CallId,
    pub state: CallState,
    pub outbound_uri: sip_core::SipUri,
    /// Caller ID rewrite mode from the selected route's gateway.
    pub caller_id_mode: Option<String>,
    /// Fixed virtual caller number when caller_id_mode is "virtual".
    pub virtual_caller: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundResponseOutcome {
    pub call_id: CallId,
    pub state: CallState,
    pub failover_uri: Option<sip_core::SipUri>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminationOutcome {
    pub call_id: CallId,
    pub state: CallState,
}

/// 并发安全的呼叫管理器：calls 用 DashMap（分片无锁），routes 用 RwLock（读多写少），
/// completed_cdrs 用 Mutex。所有方法 `&self`，可被多个 worker 并行调用。
#[derive(Debug, Default)]
pub struct CallManager {
    calls: DashMap<CallId, Call>,
    routes: RwLock<RouteTable>,
    completed_cdrs: Mutex<Vec<CallCdr>>,
}

impl CallManager {
    pub fn new(routes: RouteTable) -> Self {
        Self {
            calls: DashMap::new(),
            routes: RwLock::new(routes),
            completed_cdrs: Mutex::new(Vec::new()),
        }
    }

    pub fn update_routes(&self, routes: RouteTable) {
        *self.routes.write().expect("routes lock poisoned") = routes;
    }

    pub fn handle_inbound_invite(&self, request: &SipRequest) -> CallResult<InboundInviteOutcome> {
        let mut call = Call::from_inbound_invite(request)?;
        let call_id = call.id.clone();

        // 从 X-Call-Direction 头部读取呼叫方向（CPS 测试用）
        if let Some(dir) = request.headers.get("x-call-direction") {
            let dir_str = dir.as_str().trim().to_lowercase();
            if dir_str == "inbound" || dir_str == "outbound" {
                call.direction = dir_str;
            }
        }

        let candidates = match self
            .routes
            .read()
            .expect("routes lock poisoned")
            .select_candidates_for_direction(&call.inbound.remote_uri, &call.direction)
        {
            Ok(candidates) => candidates,
            Err(error) => {
                let _ = call.fail(None, error.to_string());
                if let Some(cdr) = CallCdr::from_completed_call(&call) {
                    self.completed_cdrs
                        .lock()
                        .expect("cdr lock poisoned")
                        .push(cdr);
                }
                self.calls.insert(call_id, call);
                return Err(error);
            }
        };

        call.candidates = candidates;
        call.current_candidate_index = 0;
        let outbound_uri = call.candidates[0].outbound_uri.clone();
        let caller_id_mode = call.candidates[0].target.caller_id_mode.clone();
        let virtual_caller = call.candidates[0].target.virtual_caller.clone();

        call.select_route(outbound_uri.clone())?;
        let state = call.state;
        self.calls.insert(call_id.clone(), call);

        Ok(InboundInviteOutcome {
            call_id,
            state,
            outbound_uri,
            caller_id_mode,
            virtual_caller,
        })
    }

    pub fn handle_inbound_invite_to_uri(
        &self,
        request: &SipRequest,
        outbound_uri: sip_core::SipUri,
    ) -> CallResult<InboundInviteOutcome> {
        let mut call = Call::from_inbound_invite(request)?;
        let call_id = call.id.clone();

        // 从 X-Call-Direction 头部读取呼叫方向（CPS 测试用）
        if let Some(dir) = request.headers.get("x-call-direction") {
            let dir_str = dir.as_str().trim().to_lowercase();
            if dir_str == "inbound" || dir_str == "outbound" {
                call.direction = dir_str;
            }
        }

        call.select_route(outbound_uri.clone())?;
        let state = call.state;
        self.calls.insert(call_id.clone(), call);

        Ok(InboundInviteOutcome {
            call_id,
            state,
            outbound_uri,
            caller_id_mode: None,
            virtual_caller: None,
        })
    }

    pub fn handle_outbound_response(
        &self,
        response: &SipResponse,
    ) -> CallResult<OutboundResponseOutcome> {
        let call_id = response
            .headers
            .get("call-id")
            .ok_or(CallError::MissingRequiredHeader("Call-ID"))?
            .as_str()
            .to_string();
        let call_id = CallId::new(call_id);

        let mut call = self
            .calls
            .get_mut(&call_id)
            .ok_or_else(|| CallError::UnknownCall(call_id.as_str().to_string()))?;

        let mut failover_uri = None;

        match response.status_code {
            100..=179 => {}
            180..=199 => call.mark_ringing()?,
            200..=299 => call.mark_answered()?,
            300..=399 => {
                let status = response.status_code;
                let redirect_uri = response
                    .headers
                    .get("contact")
                    .and_then(|v| parse_uri_from_contact(v.as_str()));
                if let Some(uri) = redirect_uri {
                    match call.redirect_to(uri) {
                        Ok(Some(target_uri)) => {
                            failover_uri = Some(target_uri);
                        }
                        _ => {
                            call.fail(Some(status), response.reason_phrase.clone())?;
                        }
                    }
                } else {
                    call.fail(Some(status), response.reason_phrase.clone())?;
                }
            }
            400..=699 => {
                let status = response.status_code;
                if (status == 408 || (500..=599).contains(&status))
                    && call.current_candidate_index + 1 < call.candidates.len()
                {
                    match call.failover_to_next() {
                        Ok(Some(uri)) => {
                            failover_uri = Some(uri);
                        }
                        _ => {
                            call.fail(Some(response.status_code), response.reason_phrase.clone())?;
                        }
                    }
                } else {
                    call.fail(Some(response.status_code), response.reason_phrase.clone())?;
                }
            }
            _ => {}
        }

        if failover_uri.is_none() {
            if let Some(cdr) = CallCdr::from_completed_call(&call) {
                self.completed_cdrs
                    .lock()
                    .expect("cdr lock poisoned")
                    .push(cdr);
            }
        }

        let state = call.state;

        Ok(OutboundResponseOutcome {
            call_id,
            state,
            failover_uri,
        })
    }

    pub fn handle_inbound_termination(
        &self,
        request: &SipRequest,
        metrics: Option<CallQualityMetrics>,
        dtmf_digits: Option<String>,
    ) -> CallResult<TerminationOutcome> {
        let call_id = request
            .headers
            .get("call-id")
            .ok_or(CallError::MissingRequiredHeader("Call-ID"))?
            .as_str()
            .to_string();
        let call_id = CallId::new(call_id);

        let mut call = self
            .calls
            .get_mut(&call_id)
            .ok_or_else(|| CallError::UnknownCall(call_id.as_str().to_string()))?;
        call.terminate()?;
        if let Some(cdr) =
            CallCdr::from_completed_call_with_metrics(&call, metrics, dtmf_digits, None)
        {
            self.completed_cdrs
                .lock()
                .expect("cdr lock poisoned")
                .push(cdr);
        }

        let state = call.state;

        Ok(TerminationOutcome { call_id, state })
    }

    pub fn get(&self, call_id: &CallId) -> Option<Call> {
        self.calls.get(call_id).map(|c| c.clone())
    }

    /// 设置录音文件路径，用于 BYE 时写入 CDR。
    pub fn set_recording_path(&self, call_id: &CallId, path: String) {
        if let Some(mut call) = self.calls.get_mut(call_id) {
            call.recording_path = Some(path);
        }
    }

    pub fn len(&self) -> usize {
        self.calls.len()
    }

    pub fn is_empty(&self) -> bool {
        self.calls.is_empty()
    }

    pub fn routes(&self) -> RouteTable {
        self.routes.read().expect("routes lock poisoned").clone()
    }

    pub fn completed_cdrs(&self) -> Vec<CallCdr> {
        self.completed_cdrs
            .lock()
            .expect("cdr lock poisoned")
            .clone()
    }

    pub fn take_completed_cdrs(&self) -> Vec<CallCdr> {
        std::mem::take(&mut *self.completed_cdrs.lock().expect("cdr lock poisoned"))
    }

    pub fn active_calls_count(&self) -> usize {
        self.calls
            .iter()
            .filter(|entry| {
                matches!(
                    entry.state,
                    crate::CallState::Routing
                        | crate::CallState::Ringing
                        | crate::CallState::Established
                )
            })
            .count()
    }

    /// 返回所有活跃呼叫的摘要（Routing/Ringing/Established）。
    pub fn active_calls(&self) -> Vec<crate::ActiveCall> {
        self.calls
            .iter()
            .filter(|entry| {
                matches!(
                    entry.state,
                    crate::CallState::Routing
                        | crate::CallState::Ringing
                        | crate::CallState::Established
                )
            })
            .map(|entry| crate::ActiveCall {
                call_id: entry.id.as_str().to_string(),
                caller: entry.caller.clone(),
                callee: entry.inbound.remote_uri.user.clone(),
                state: entry.state.as_str().to_string(),
                started_at_ms: sys_millis(entry.started_at),
                answered_at_ms: entry.answered_at.map(sys_millis),
                gateway: entry
                    .outbound
                    .as_ref()
                    .map(|leg| leg.remote_uri.host.clone()),
            })
            .collect()
    }

    /// Forcibly terminates a call by its Call-ID string, moving it to Failed
    /// state and archiving any CDR. Used by the session timer watchdog.
    pub fn terminate_call(&self, call_id: &str) {
        let cid = crate::CallId::new(call_id.to_string());
        if let Some(mut call) = self.calls.get_mut(&cid) {
            let _ = call.fail(None, "Session-Expires timeout".to_string());
            if let Some(cdr) = crate::cdr::CallCdr::from_completed_call(&call) {
                self.completed_cdrs
                    .lock()
                    .expect("cdr lock poisoned")
                    .push(cdr);
            }
        }
    }
}

fn sys_millis(t: std::time::SystemTime) -> i64 {
    t.duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn parse_uri_from_contact(raw: &str) -> Option<sip_core::SipUri> {
    let value = raw.trim();
    let uri_raw = if let Some(start) = value.find('<') {
        let end = value[start + 1..].find('>')?;
        &value[start + 1..start + 1 + end]
    } else {
        value.split(';').next().unwrap_or(value).trim()
    };
    std::str::FromStr::from_str(uri_raw).ok()
}
