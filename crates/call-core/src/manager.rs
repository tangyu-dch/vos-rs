//! # 呼叫管理器
//!
//! 本模块实现了 VoIP 呼叫的核心状态管理，包括：
//!
//! - **INVITE 处理**：接收入站 INVITE，选择路由，生成出站 INVITE
//! - **响应处理**：处理出站响应（100/180/200/3xx/4xx/5xx），更新呼叫状态
//! - **Failover**：408/5xx 响应自动切换到下一个候选网关
//! - **终止处理**：BYE/CANCEL/超时处理，生成 CDR
//! - **并发安全**：使用 `DashMap`（分片无锁）存储呼叫状态
//!
//! ## 呼叫生命周期
//!
//! ```text
//! INVITE → Routing → Ringing → Established → Terminated
//!                    ↓              ↓
//!                  Failed         Failed
//! ```
//!
//! ## 网关容量控制
//!
//! 每次成功选定网关后调用 `increment_active`，
//! 呼叫结束（BYE/CANCEL/超时/failover）时调用 `decrement_active`。

use crate::{
    Call, CallCdr, CallDirection, CallError, CallEvent, CallId, CallQualityMetrics, CallResult,
    CallSource, CallState, CallerIdentity, CallerNumberDirectory, GatewayHealthTracker,
    OutboundPolicyDirectory, RouteTable, SelectedRoute, WebhookEvent, WEBHOOK_SCHEMA_VERSION,
};
use arc_swap::ArcSwap;
use dashmap::DashMap;
use sip_core::{SipRequest, SipResponse};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

/// Webhook 事件输出通道抽象。
///
/// 实现必须立即返回，禁止在呼叫热路径执行网络或持久化操作。
pub trait CallEventSink: Send + Sync + std::fmt::Debug {
    /// 尝试投递事件到有界异步队列。
    fn try_send_event(&self, event: WebhookEvent) -> Result<(), CallEventSendError>;
}

/// Webhook 事件进入异步队列时的失败原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallEventSendError {
    /// 队列已满。
    QueueFull,
    /// 消费者已经退出。
    ConsumerClosed,
}

impl CallEventSink for tokio::sync::mpsc::Sender<WebhookEvent> {
    fn try_send_event(&self, event: WebhookEvent) -> Result<(), CallEventSendError> {
        self.try_send(event).map_err(|error| match error {
            tokio::sync::mpsc::error::TrySendError::Full(_) => CallEventSendError::QueueFull,
            tokio::sync::mpsc::error::TrySendError::Closed(_) => CallEventSendError::ConsumerClosed,
        })
    }
}

/// CDR 输出通道抽象。
///
/// 生产环境使用有界 `Sender`，测试和兼容调用方仍可使用无界 Sender。
/// 呼叫热路径只尝试投递，不等待慢速数据库，从而避免阻塞 SIP 处理线程。
pub trait CdrSink: Send + Sync + std::fmt::Debug {
    /// 尝试投递一条 CDR；返回错误表示队列已满或消费者已退出。
    fn try_send_cdr(&self, cdr: CallCdr) -> Result<(), CdrSendError>;
}

/// CDR 投递失败原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CdrSendError {
    /// 有界队列已满，生产者不能阻塞等待。
    QueueFull,
    /// CDR 消费者已经退出。
    ConsumerClosed,
}

impl CdrSink for tokio::sync::mpsc::Sender<CallCdr> {
    fn try_send_cdr(&self, cdr: CallCdr) -> Result<(), CdrSendError> {
        self.try_send(cdr).map_err(|error| match error {
            tokio::sync::mpsc::error::TrySendError::Full(_) => CdrSendError::QueueFull,
            tokio::sync::mpsc::error::TrySendError::Closed(_) => CdrSendError::ConsumerClosed,
        })
    }
}

impl CdrSink for tokio::sync::mpsc::UnboundedSender<CallCdr> {
    fn try_send_cdr(&self, cdr: CallCdr) -> Result<(), CdrSendError> {
        self.send(cdr).map_err(|_| CdrSendError::ConsumerClosed)
    }
}

/// 入站 INVITE 处理结果。
///
/// 当 SIP 层收到入站 INVITE 时，调用 `CallManager::handle_inbound_invite` 处理。
/// 返回结果包含呼叫 ID、状态、出站 URI 和 Caller ID 重写配置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboundInviteOutcome {
    /// 呼叫 ID（SIP Call-ID）
    pub call_id: CallId,
    /// 当前呼叫状态（通常为 Routing）
    pub state: CallState,
    /// 出站 SIP URI（已应用前缀规则）
    pub outbound_uri: sip_core::SipUri,
    /// Caller ID 重写模式（passthrough/virtual/random）
    pub caller_id_mode: Option<String>,
    /// 固定虚拟主叫号码（当 caller_id_mode 为 "virtual" 时使用）
    pub virtual_caller: Option<String>,
    /// Resolved caller number and its immutable owner gateway.
    pub caller_identity: Option<CallerIdentity>,
}

/// 出站响应处理结果。
///
/// 当收到网关的出站响应时，调用 `CallManager::handle_outbound_response` 处理。
/// 返回结果包含呼叫 ID、状态、failover URI 和当前网关 ID。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundResponseOutcome {
    /// 呼叫 ID
    pub call_id: CallId,
    /// 当前呼叫状态
    pub state: CallState,
    /// Failover URI（如果需要切换网关）
    pub failover_uri: Option<sip_core::SipUri>,
    /// 返回当前响应的网关 ID（用于健康状态更新）
    pub gateway_id: String,
    /// 发生故障切换时的新网关 ID。
    pub failover_gateway_id: Option<String>,
    /// Caller identity to reuse on a same-gateway failover attempt.
    pub caller_identity: Option<CallerIdentity>,
}

/// 呼叫终止结果。
///
/// 当收到入站 BYE 或呼叫超时时，调用 `CallManager::handle_inbound_termination` 处理。
/// 返回结果包含呼叫 ID 和最终状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminationOutcome {
    /// 呼叫 ID
    pub call_id: CallId,
    /// 最终呼叫状态（通常为 Terminated）
    pub state: CallState,
}

/// 并发安全的呼叫管理器。
///
/// 核心职责：
/// - 管理所有活跃呼叫的状态（Routing/Ringing/Established/Failed/Terminated）
/// - 处理入站 INVITE：选择路由、生成出站 INVITE
/// - 处理出站响应：更新状态、触发 Failover
/// - 处理 BYE/CANCEL：终止呼叫、生成 CDR
///
/// 并发模型：
/// - `calls`：使用 `DashMap`（分片无锁 HashMap），支持高并发读写
/// - `routes`：使用 `RwLock`（读多写少），路由表热更新时写锁
/// - `cdr_sender`：异步通道，CDR 异步写入数据库
///
/// 所有方法接受 `&self`，可被多个 worker 并行调用。
#[derive(Debug)]
pub struct CallManager {
    /// 活跃呼叫表（Call-ID → Call）
    calls: DashMap<CallId, Call>,
    /// 路由表（支持热更新）
    routes: ArcSwap<RouteTable>,
    caller_numbers: ArcSwap<CallerNumberDirectory>,
    outbound_policies: ArcSwap<OutboundPolicyDirectory>,
    /// CDR 异步写入通道
    cdr_sender: Arc<dyn CdrSink>,
    cdr_dropped: AtomicU64,
    event_sink: Option<Arc<dyn CallEventSink>>,
    event_sequence: AtomicU64,
    event_dropped: AtomicU64,
    /// 活跃呼叫计数的性能缓存（避免高并发下对 DashMap 频繁遍历产生严重的锁竞争）
    active_calls_cache: std::sync::atomic::AtomicUsize,
    active_calls_last_update: std::sync::atomic::AtomicU64,
}

impl CallManager {
    pub fn new<S: CdrSink + 'static>(routes: RouteTable, cdr_sender: S) -> Self {
        Self::new_inner(routes, cdr_sender, None)
    }

    /// 创建启用异步 Webhook 事件输出的呼叫管理器。
    pub fn new_with_event_sink<S, E>(routes: RouteTable, cdr_sender: S, event_sink: E) -> Self
    where
        S: CdrSink + 'static,
        E: CallEventSink + 'static,
    {
        Self::new_inner(routes, cdr_sender, Some(Arc::new(event_sink)))
    }

    fn new_inner<S: CdrSink + 'static>(
        routes: RouteTable,
        cdr_sender: S,
        event_sink: Option<Arc<dyn CallEventSink>>,
    ) -> Self {
        Self {
            calls: DashMap::new(),
            routes: ArcSwap::new(Arc::new(routes)),
            caller_numbers: ArcSwap::new(Arc::new(CallerNumberDirectory::default())),
            outbound_policies: ArcSwap::new(Arc::new(OutboundPolicyDirectory::default())),
            cdr_sender: Arc::new(cdr_sender),
            cdr_dropped: AtomicU64::new(0),
            event_sink,
            event_sequence: AtomicU64::new(0),
            event_dropped: AtomicU64::new(0),
            active_calls_cache: std::sync::atomic::AtomicUsize::new(0),
            active_calls_last_update: std::sync::atomic::AtomicU64::new(0),
        }
    }

    pub fn owns_number(&self, number: &str, gateway_id: &str) -> bool {
        self.caller_numbers.load().owns_number(number, gateway_id)
    }

    fn push_event(&self, call_id: &CallId, event: CallEvent) {
        let Some(sink) = &self.event_sink else {
            return;
        };
        let envelope = WebhookEvent {
            event_id: uuid::Uuid::new_v4().to_string(),
            schema_version: WEBHOOK_SCHEMA_VERSION.to_string(),
            call_id: call_id.as_str().to_string(),
            sequence: self.event_sequence.fetch_add(1, Ordering::Relaxed) + 1,
            occurred_at_ms: sys_millis(std::time::SystemTime::now()),
            event,
        };
        if sink.try_send_event(envelope).is_err() {
            self.event_dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn push_cdr(&self, cdr: CallCdr) {
        if self.cdr_sender.try_send_cdr(cdr).is_err() {
            self.cdr_dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// 返回因 CDR 队列满或消费者退出而未能投递的数量。
    pub fn dropped_cdr_count(&self) -> u64 {
        self.cdr_dropped.load(Ordering::Relaxed)
    }

    /// 返回因事件队列满或消费者退出而丢弃的 Webhook 事件数。
    pub fn dropped_event_count(&self) -> u64 {
        self.event_dropped.load(Ordering::Relaxed)
    }

    pub fn update_routes(&self, routes: RouteTable) {
        self.routes.store(Arc::new(routes));
    }

    /// Atomically replaces caller-number ownership used by new calls.
    pub fn update_caller_numbers(&self, directory: CallerNumberDirectory) {
        self.caller_numbers.store(Arc::new(directory));
    }

    /// Atomically replaces source-owned outbound policies used by new calls.
    pub fn update_outbound_policies(&self, mut directory: OutboundPolicyDirectory) {
        directory.inherit_selection_state(&self.outbound_policies.load());
        self.outbound_policies.store(Arc::new(directory));
    }

    pub fn handle_inbound_invite(&self, request: &SipRequest) -> CallResult<InboundInviteOutcome> {
        self.handle_inbound_invite_with_health(request, None)
    }

    /// Handles an INVITE while applying the gateway circuit breaker.
    pub fn handle_inbound_invite_with_health(
        &self,
        request: &SipRequest,
        health: Option<&GatewayHealthTracker>,
    ) -> CallResult<InboundInviteOutcome> {
        self.handle_inbound_invite_with_source_and_health(request, None, health)
    }

    /// Handles an INVITE for an authenticated extension or access trunk source.
    pub fn handle_inbound_invite_with_source_and_health(
        &self,
        request: &SipRequest,
        source: Option<&CallSource>,
        health: Option<&GatewayHealthTracker>,
    ) -> CallResult<InboundInviteOutcome> {
        self.handle_inbound_invite_with_source_and_health_and_direction(
            request,
            source,
            health,
            CallDirection::Outbound,
        )
    }

    /// Handles an INVITE with a direction derived from trusted ingress identity.
    pub fn handle_inbound_invite_with_source_and_health_and_direction(
        &self,
        request: &SipRequest,
        source: Option<&CallSource>,
        health: Option<&GatewayHealthTracker>,
        direction: CallDirection,
    ) -> CallResult<InboundInviteOutcome> {
        let mut call = Call::from_inbound_invite_with_direction(request, direction)?;
        let call_id = call.id.clone();

        self.push_event(
            &call_id,
            CallEvent::CallInitiated {
                caller: call.caller.clone(),
                callee: call.inbound.remote_uri.user.as_ref().map(|u| u.to_string()),
                direction: call.direction.clone(),
                leg: "a_leg".to_string(),
            },
        );

        let policy_candidates = source
            .map(|source| {
                self.outbound_policies.load().select_source_candidates(
                    source,
                    &call.inbound.remote_uri,
                    &call.direction,
                    health,
                )
            })
            .transpose()?
            .flatten();
        let candidates = if let Some(candidates) = policy_candidates {
            Ok(candidates)
        } else {
            let routes = self.routes.load();
            match health {
                Some(health) => routes.select_healthy_candidates(
                    &call.inbound.remote_uri,
                    health,
                    Some(&call.direction),
                ),
                None => routes
                    .select_candidates_for_direction(&call.inbound.remote_uri, &call.direction),
            }
        };
        let mut candidates = match candidates {
            Ok(candidates) => candidates,
            Err(error) => {
                let reason = error.to_string();
                let _ = call.fail(None, reason.clone());
                if let Some(cdr) = CallCdr::from_completed_call(&call) {
                    self.push_cdr(cdr);
                }
                self.calls.insert(call_id.clone(), call);
                self.push_event(
                    &call_id,
                    CallEvent::CallFinished {
                        duration_secs: 0,
                        sip_status: None,
                        q850_cause: None,
                        reason,
                        leg: "a_leg".to_string(),
                    },
                );
                return Err(error);
            }
        };

        let original_number = caller_number_from_request(request).ok_or_else(|| {
            CallError::CallerIdentityUnavailable("inbound From has no caller number".to_string())
        })?;
        let source_resolution = source
            .map(|source| {
                self.outbound_policies.load().resolve_with_alternatives(
                    source,
                    &original_number,
                    request.uri.user.as_deref().unwrap_or(""),
                    &candidates,
                    call_id.as_str(),
                )
            })
            .transpose()?
            .flatten();
        if let Some(source) = source {
            call.audit = self.outbound_policies.load().audit_snapshot(source);
        }
        let caller_identity = if let Some(mut selections) = source_resolution {
            let (identity, source_candidates) = selections.remove(0);
            candidates = source_candidates;
            call.caller_identity_alternatives = selections;
            Some(identity)
        } else {
            let policy_target = &candidates[0].target;
            let identity = self.caller_numbers.load().resolve(
                policy_target.caller_id_mode.as_deref(),
                policy_target.virtual_caller.as_deref(),
                &original_number,
                &candidates,
                call_id.as_str(),
            )?;
            if let Some(identity) = &identity {
                candidates
                    .retain(|candidate| candidate.target.gateway_id == identity.owner_gateway_id);
            }
            identity
        };
        call.audit.original_caller = Some(original_number.clone());
        call.audit.presented_caller = Some(
            caller_identity
                .as_ref()
                .map_or(original_number, |identity| {
                    identity.presented_number.clone()
                }),
        );
        call.caller_identity = caller_identity.clone();
        call.candidates = candidates;
        call.current_candidate_index = 0;
        let outbound_uri = call.candidates[0].outbound_uri.clone();
        let caller_id_mode = call.candidates[0].target.caller_id_mode.clone();
        let virtual_caller = call.candidates[0].target.virtual_caller.clone();
        if call.audit.caller_mode.is_none() {
            call.audit.caller_mode.clone_from(&caller_id_mode);
            call.audit.caller_selection.clone_from(&caller_id_mode);
        }

        call.select_route(outbound_uri.clone())?;
        let state = call.state;
        self.calls.insert(call_id.clone(), call);

        Ok(InboundInviteOutcome {
            call_id,
            state,
            outbound_uri,
            caller_id_mode,
            virtual_caller,
            caller_identity,
        })
    }

    pub fn handle_inbound_invite_to_uri(
        &self,
        request: &SipRequest,
        outbound_uri: sip_core::SipUri,
    ) -> CallResult<InboundInviteOutcome> {
        self.handle_inbound_invite_to_uri_with_direction(
            request,
            outbound_uri,
            CallDirection::Outbound,
        )
    }

    /// Handles a pre-routed INVITE with a trusted business direction.
    pub fn handle_inbound_invite_to_uri_with_direction(
        &self,
        request: &SipRequest,
        outbound_uri: sip_core::SipUri,
        direction: CallDirection,
    ) -> CallResult<InboundInviteOutcome> {
        let mut call = Call::from_inbound_invite_with_direction(request, direction)?;
        let call_id = call.id.clone();

        self.push_event(
            &call_id,
            CallEvent::CallInitiated {
                caller: call.caller.clone(),
                callee: call.inbound.remote_uri.user.as_ref().map(|u| u.to_string()),
                direction: call.direction.clone(),
                leg: "a_leg".to_string(),
            },
        );

        call.select_route(outbound_uri.clone())?;
        let state = call.state;
        self.calls.insert(call_id.clone(), call);

        Ok(InboundInviteOutcome {
            call_id,
            state,
            outbound_uri,
            caller_id_mode: None,
            virtual_caller: None,
            caller_identity: None,
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

        let previous_state = call.state;
        let responding_gateway_id = call
            .candidates
            .get(call.current_candidate_index)
            .map(|candidate| candidate.target.gateway_id.as_str().to_string())
            .unwrap_or_default();
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
                self.push_cdr(cdr);
            }
        }

        let state = call.state;
        let lifecycle_event = match (previous_state, state) {
            (previous, CallState::Ringing) if previous != CallState::Ringing => {
                Some(CallEvent::CallRinging {
                    sip_status: response.status_code,
                    leg: "b_leg".to_string(),
                })
            }
            (previous, CallState::Established) if previous != CallState::Established => {
                Some(CallEvent::CallAnswered {
                    sip_status: response.status_code,
                    leg: "b_leg".to_string(),
                })
            }
            (previous, CallState::Failed) if previous != CallState::Failed => {
                Some(CallEvent::CallFinished {
                    duration_secs: answered_duration_secs(&call),
                    sip_status: call
                        .failure_cause
                        .as_ref()
                        .and_then(|cause| cause.status_code),
                    q850_cause: None,
                    reason: call
                        .failure_cause
                        .as_ref()
                        .map(|cause| cause.reason.clone())
                        .unwrap_or_else(|| "呼叫失败".to_string()),
                    leg: "b_leg".to_string(),
                })
            }
            _ => None,
        };
        let failover_gateway_id = failover_uri.as_ref().and_then(|_| {
            call.candidates
                .get(call.current_candidate_index)
                .map(|candidate| candidate.target.gateway_id.as_str().to_string())
        });
        let outcome = OutboundResponseOutcome {
            call_id: call_id.clone(),
            state,
            failover_uri,
            gateway_id: responding_gateway_id,
            failover_gateway_id,
            caller_identity: call.caller_identity.clone(),
        };
        drop(call);
        if let Some(event) = lifecycle_event {
            self.push_event(&call_id, event);
        }
        Ok(outcome)
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
            self.push_cdr(cdr);
        }

        let state = call.state;
        let duration_secs = answered_duration_secs(&call);
        drop(call);
        self.push_event(
            &call_id,
            CallEvent::CallFinished {
                duration_secs,
                sip_status: None,
                q850_cause: None,
                reason: "通话结束".to_string(),
                leg: "a_leg".to_string(),
            },
        );

        Ok(TerminationOutcome { call_id, state })
    }

    pub fn get(&self, call_id: &CallId) -> Option<Call> {
        self.calls.get(call_id).map(|c| c.clone())
    }

    /// Advances a virtual caller pool after the selected number reaches capacity.
    ///
    /// The caller identity and its owner-gateway routes are replaced while holding the
    /// call entry lock, so readers never observe a number paired with another owner.
    pub fn advance_caller_pool(&self, call_id: &CallId) -> Option<InboundInviteOutcome> {
        let mut call = self.calls.get_mut(call_id)?;
        if call.state != CallState::Routing {
            return None;
        }
        let (identity, candidates) = call.caller_identity_alternatives.first()?.clone();
        let selected = candidates.first()?.clone();
        call.caller_identity_alternatives.remove(0);
        call.audit.presented_caller = Some(identity.presented_number.clone());
        call.audit.fallback_used = true;
        call.caller_identity = Some(identity.clone());
        call.candidates = candidates;
        call.current_candidate_index = 0;
        let outbound_uri = selected.outbound_uri;
        if let Some(outbound) = call.outbound.as_mut() {
            outbound.remote_uri = outbound_uri.clone();
        }
        Some(InboundInviteOutcome {
            call_id: call_id.clone(),
            state: call.state,
            outbound_uri,
            caller_id_mode: selected.target.caller_id_mode,
            virtual_caller: selected.target.virtual_caller,
            caller_identity: Some(identity),
        })
    }

    /// 设置录音文件路径，用于 BYE 时写入 CDR。
    pub fn set_recording_path(&self, call_id: &CallId, path: String) {
        if let Some(mut call) = self.calls.get_mut(call_id) {
            call.recording_path = Some(path);
        }
    }

    /// Freezes the billing account selected from the authenticated call source.
    pub fn set_billing_account(&self, call_id: &CallId, account: Option<String>) {
        if let Some(mut call) = self.calls.get_mut(call_id) {
            call.billing_account = account;
        }
    }

    /// 设置呼叫的候选路由，供并行振铃/分机组使用。
    pub fn set_candidates(&self, call_id: &CallId, candidates: Vec<SelectedRoute>) {
        if let Some(mut call) = self.calls.get_mut(call_id) {
            call.candidates = candidates;
        }
    }

    /// Freezes ingress and pulse-rating facts selected before the call starts.
    pub fn set_cdr_audit_context(
        &self,
        call_id: &CallId,
        ingress_trunk_id: Option<String>,
        billing_interval_secs: Option<u32>,
        price_per_interval: Option<f64>,
    ) {
        if let Some(mut call) = self.calls.get_mut(call_id) {
            if ingress_trunk_id.is_some() {
                call.audit.ingress_trunk_id = ingress_trunk_id;
            }
            call.audit.billing_interval_secs = billing_interval_secs;
            call.audit.price_per_interval = price_per_interval;
        }
    }

    pub fn len(&self) -> usize {
        self.calls.len()
    }

    pub fn is_empty(&self) -> bool {
        self.calls.is_empty()
    }

    pub fn routes(&self) -> RouteTable {
        (**self.routes.load()).clone()
    }

    pub fn active_calls_count(&self) -> usize {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let last_update = self.active_calls_last_update.load(Ordering::Relaxed);

        // Cache active call counts for 500ms under high load to eliminate DashMap iterations.
        if now >= last_update && now - last_update < 500 {
            return self.active_calls_cache.load(Ordering::Relaxed);
        }

        let count = self
            .calls
            .iter()
            .filter(|entry| {
                matches!(
                    entry.state,
                    crate::CallState::Routing
                        | crate::CallState::Ringing
                        | crate::CallState::Established
                )
            })
            .count();

        self.active_calls_cache.store(count, Ordering::Relaxed);
        self.active_calls_last_update.store(now, Ordering::Relaxed);
        count
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
                callee: entry
                    .inbound
                    .remote_uri
                    .user
                    .as_ref()
                    .map(|u| u.to_string()),
                state: entry.state.as_str().to_string(),
                started_at_ms: sys_millis(entry.started_at),
                answered_at_ms: entry.answered_at.map(sys_millis),
                gateway: entry
                    .outbound
                    .as_ref()
                    .map(|leg| leg.remote_uri.host.to_string()),
            })
            .collect()
    }

    /// Forcibly terminates a call by its Call-ID string, moving it to Failed
    /// state and archiving any CDR. Used by the session timer watchdog.
    pub fn terminate_call(&self, call_id: &str) {
        self.terminate_call_with_reason(call_id, "Session-Expires timeout");
    }

    /// Forcibly terminates a call with a specific failure reason.
    pub fn terminate_call_with_reason(&self, call_id: &str, reason: &str) {
        self.try_terminate_call_with_reason(call_id, reason);
    }

    /// Forcibly terminates a call once and reports whether this invocation won the
    /// termination race. Callers can use the return value to avoid duplicate
    /// settlement when multiple lifecycle paths observe the same call ending.
    pub fn try_terminate_call_with_reason(&self, call_id: &str, reason: &str) -> bool {
        let cid = crate::CallId::new(call_id.to_string());
        if let Some(mut call) = self.calls.get_mut(&cid) {
            let result = if call.answered_at.is_some() {
                call.terminate()
            } else {
                call.fail(None, reason.to_string())
            };
            if result.is_err() {
                return false;
            }
            if let Some(cdr) = crate::cdr::CallCdr::from_completed_call(&call) {
                self.push_cdr(cdr);
            }
            let duration_secs = answered_duration_secs(&call);
            drop(call);
            self.push_event(
                &cid,
                CallEvent::CallFinished {
                    duration_secs,
                    sip_status: None,
                    q850_cause: None,
                    reason: reason.to_string(),
                    leg: "a_leg".to_string(),
                },
            );
            return true;
        }
        false
    }

    /// Returns the gateway_id of the currently selected route for a call, if any.
    pub fn current_gateway_id(&self, call_id: &str) -> Option<String> {
        let cid = crate::CallId::new(call_id.to_string());
        self.calls.get(&cid).and_then(|call| {
            call.candidates
                .get(call.current_candidate_index)
                .map(|c| c.target.gateway_id.as_str().to_string())
        })
    }
}

fn sys_millis(t: std::time::SystemTime) -> i64 {
    t.duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn answered_duration_secs(call: &Call) -> u64 {
    call.answered_at
        .and_then(|answered_at| {
            call.ended_at
                .unwrap_or_else(std::time::SystemTime::now)
                .duration_since(answered_at)
                .ok()
        })
        .map_or(0, |duration| duration.as_secs())
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

fn caller_number_from_request(request: &SipRequest) -> Option<String> {
    let header = request.headers.get("from")?.as_str().trim();
    let sip_start = header.find("sip:").map(|index| index + 4)?;
    let user = header[sip_start..].split(['@', ';', '>']).next()?.trim();
    (!user.is_empty()).then(|| user.to_string())
}
