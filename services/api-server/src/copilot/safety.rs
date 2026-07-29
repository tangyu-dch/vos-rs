//! Copilot 工具风险分级、待审批动作和审批 API。

use axum::{
    extract::{Extension, Path, State},
    Json,
};
use cdr_core::CopilotAction;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    copilot::TelecomCopilotEngine,
    system::{auth::Claims, permissions::required_permission},
    ApiError, AppState,
};

/// 工具执行风险级别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolRisk {
    ReadOnly,
    Write,
    HighRisk,
}

impl ToolRisk {
    fn persisted_name(self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::Write => "write",
            Self::HighRisk => "high_risk",
        }
    }
}

/// 工具固定授权目标。审批时据此复用 HTTP RBAC，不信任客户端传入的路径。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolPolicy {
    pub risk: ToolRisk,
    pub method: &'static str,
    pub path: &'static str,
}

/// 返回工具的风险分级与权限目标；未知工具一律拒绝。
pub fn tool_policy(name: &str) -> Option<ToolPolicy> {
    let policy = match name {
        "vos_get_dashboard_stats" => read("/api/dashboard/stats"),
        "vos_get_daily_report" => read("/api/reports/daily"),
        "vos_list_cdrs" | "vos_export_cdrs" => read("/api/cdrs"),
        "vos_get_sip_flows" | "vos_list_active_calls" => read("/api/v1/calls"),
        "vos_list_registrations" => read("/api/registrations"),
        "vos_list_gateways" | "vos_export_gateways" => read("/api/gateways"),
        "vos_preview_route" => read("/api/route-preview"),
        "vos_list_anti_fraud_rules" => read("/api/anti-fraud/rules"),
        "vos_list_extensions" | "vos_export_extensions" => read("/api/users"),
        "vos_list_ivr_menus" => read("/api/v1/ivr/menus"),
        "vos_list_routes" | "vos_export_routes" => read("/api/routes"),
        "vos_list_billing_accounts" | "vos_export_billing_accounts" => read("/api/accounts"),
        "vos_list_rates" | "vos_export_rates" => read("/api/rates"),
        "vos_create_extension" => write("POST", "/api/users"),
        "vos_create_ivr_menu" | "vos_add_ivr_node" => write("POST", "/api/v1/ivr/menus"),
        "vos_create_gateway" => write("POST", "/api/gateways"),
        "vos_create_route" => write("POST", "/api/routes"),
        "vos_upsert_rate" => write("POST", "/api/rates"),
        "vos_create_anti_fraud_rule" => write("POST", "/api/anti-fraud/rules"),
        "vos_terminate_call" => high_risk("POST", "/api/calls/id/terminate"),
        "vos_delete_extension" => high_risk("DELETE", "/api/users/id"),
        "vos_delete_ivr_menu" => high_risk("DELETE", "/api/v1/ivr/menus/id"),
        "vos_delete_gateway" => high_risk("DELETE", "/api/gateways/id"),
        "vos_delete_route" => high_risk("DELETE", "/api/routes/id"),
        "vos_recharge_billing_account" => high_risk("POST", "/api/accounts/id/credit"),
        "vos_delete_rate" => high_risk("DELETE", "/api/rates/id"),
        "vos_delete_anti_fraud_rule" => high_risk("DELETE", "/api/anti-fraud/rules/id"),
        "vos_import_extensions" => high_risk("POST", "/api/v1/extensions/import"),
        "vos_import_gateways" => high_risk("POST", "/api/gateways/import"),
        "vos_import_routes" => high_risk("POST", "/api/routes/import"),
        "vos_import_rates" => high_risk("POST", "/api/rates/import"),
        _ => return None,
    };
    Some(policy)
}

const fn read(path: &'static str) -> ToolPolicy {
    ToolPolicy {
        risk: ToolRisk::ReadOnly,
        method: "GET",
        path,
    }
}

const fn write(method: &'static str, path: &'static str) -> ToolPolicy {
    ToolPolicy {
        risk: ToolRisk::Write,
        method,
        path,
    }
}

const fn high_risk(method: &'static str, path: &'static str) -> ToolPolicy {
    ToolPolicy {
        risk: ToolRisk::HighRisk,
        method,
        path,
    }
}

/// 创建固定工具名和参数的审批提案。
pub async fn propose_action(
    state: &AppState,
    session_id: &str,
    claims: &Claims,
    tool_name: &str,
    tool_arguments: &Value,
    policy: ToolPolicy,
) -> Result<CopilotAction, String> {
    if policy.risk == ToolRisk::ReadOnly {
        return Err("只读工具无需创建审批动作".to_string());
    }
    if !tool_allowed(state, claims, policy).await {
        return Err(format!("当前角色无权申请执行工具 {tool_name}"));
    }
    let now = OffsetDateTime::now_utc();
    let action = CopilotAction {
        id: format!("cpa-{}", Uuid::new_v4().simple()),
        session_id: session_id.to_string(),
        operator: claims.sub.clone(),
        requested_role: claims.role.clone(),
        tool_name: tool_name.to_string(),
        tool_arguments: tool_arguments.clone(),
        risk_level: policy.risk.persisted_name().to_string(),
        status: "pending".to_string(),
        reviewed_by: None,
        reviewed_role: None,
        review_note: None,
        result: None,
        created_at: now,
        reviewed_at: None,
        completed_at: None,
    };
    state
        .store
        .create_copilot_action(&action)
        .await
        .map_err(|error| format!("保存待审批动作失败: {error}"))
}

/// 列出当前操作员在指定会话中的工具动作。
pub async fn list_actions(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(session_id): Path<String>,
) -> Result<Json<Vec<CopilotAction>>, ApiError> {
    ensure_session_owner(&state, &session_id, &claims).await?;
    let actions = state
        .store
        .list_copilot_actions(&session_id, &claims.sub)
        .await
        .map_err(|error| ApiError::internal(format!("查询审批动作失败: {error}")))?;
    Ok(Json(actions))
}

/// 审批并执行已持久化的固定工具动作。
pub async fn approve_action(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path((session_id, action_id)): Path<(String, String)>,
) -> Result<Json<CopilotAction>, ApiError> {
    ensure_session_owner(&state, &session_id, &claims).await?;
    let pending = get_action(&state, &session_id, &action_id, &claims).await?;
    let policy = tool_policy(&pending.tool_name)
        .filter(|item| item.risk != ToolRisk::ReadOnly)
        .ok_or_else(|| ApiError::forbidden("无权审批未知或只读工具"))?;
    if !tool_allowed(&state, &claims, policy).await {
        return Err(ApiError::forbidden("当前角色无权执行该审批动作"));
    }
    let claimed = state
        .store
        .claim_copilot_action(&action_id, &session_id, &claims.sub, &claims.role)
        .await
        .map_err(|error| ApiError::internal(format!("领取审批动作失败: {error}")))?
        .ok_or_else(|| ApiError::bad_request("参数无效：动作已处理或不再待审批"))?;
    let engine = TelecomCopilotEngine::new(&state, None);
    let result = engine
        .execute_tool(&claimed.tool_name, &claimed.tool_arguments)
        .await;
    let succeeded = tool_result_succeeded(&result);
    let completed = state
        .store
        .complete_copilot_action(&action_id, succeeded, &result)
        .await
        .map_err(|error| ApiError::internal(format!("保存审批结果失败: {error}")))?
        .ok_or_else(|| ApiError::internal("审批动作状态异常"))?;
    Ok(Json(completed))
}

fn tool_result_succeeded(result: &Value) -> bool {
    result
        .get("success")
        .and_then(Value::as_bool)
        .unwrap_or_else(|| result.get("error").is_none())
}

#[derive(Debug, Deserialize)]
pub struct RejectActionRequest {
    pub note: Option<String>,
}

/// 拒绝待审批动作，不执行任何业务工具。
pub async fn reject_action(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path((session_id, action_id)): Path<(String, String)>,
    payload: Option<Json<RejectActionRequest>>,
) -> Result<Json<CopilotAction>, ApiError> {
    ensure_session_owner(&state, &session_id, &claims).await?;
    let pending = get_action(&state, &session_id, &action_id, &claims).await?;
    let policy = tool_policy(&pending.tool_name)
        .ok_or_else(|| ApiError::forbidden("无权拒绝未知工具动作"))?;
    if !tool_allowed(&state, &claims, policy).await {
        return Err(ApiError::forbidden("当前角色无权处理该审批动作"));
    }
    let note = payload.and_then(|Json(value)| value.note);
    let rejected = state
        .store
        .reject_copilot_action(
            &action_id,
            &session_id,
            &claims.sub,
            &claims.role,
            note.as_deref(),
        )
        .await
        .map_err(|error| ApiError::internal(format!("拒绝审批动作失败: {error}")))?
        .ok_or_else(|| ApiError::bad_request("参数无效：动作已处理或不再待审批"))?;
    Ok(Json(rejected))
}

/// 使用工具对应的真实接口权限验证当前数据库会话。
pub async fn tool_allowed(state: &AppState, claims: &Claims, policy: ToolPolicy) -> bool {
    let Some(permission) = required_permission(policy.method, policy.path) else {
        return false;
    };
    state.access_snapshot.read().await.allows(
        &claims.sub,
        &claims.role,
        claims.auth_version,
        permission,
    )
}

async fn ensure_session_owner(
    state: &AppState,
    session_id: &str,
    claims: &Claims,
) -> Result<(), ApiError> {
    state
        .store
        .get_copilot_session(session_id, &claims.sub)
        .await
        .map_err(|error| ApiError::internal(format!("查询 Copilot 会话失败: {error}")))?
        .ok_or_else(|| ApiError::not_found("会话不存在"))?;
    Ok(())
}

async fn get_action(
    state: &AppState,
    session_id: &str,
    action_id: &str,
    claims: &Claims,
) -> Result<CopilotAction, ApiError> {
    state
        .store
        .get_copilot_action(action_id, session_id, &claims.sub)
        .await
        .map_err(|error| ApiError::internal(format!("查询审批动作失败: {error}")))?
        .ok_or_else(|| ApiError::not_found("审批动作不存在"))
}

/// 返回给模型的审批占位结果，明确写操作尚未执行。
pub fn approval_tool_result(action: &CopilotAction) -> Value {
    json!({
        "approval_required": true,
        "executed": false,
        "action_id": action.id,
        "risk_level": action.risk_level,
        "status": action.status,
        "message": "操作尚未执行，必须由当前登录操作员显式审批"
    })
}

#[cfg(test)]
mod tests {
    use super::{tool_policy, tool_result_succeeded, ToolRisk};
    use crate::system::permissions::required_permission;
    use serde_json::json;

    #[test]
    fn destructive_and_financial_tools_are_high_risk() {
        for name in [
            "vos_terminate_call",
            "vos_delete_extension",
            "vos_recharge_billing_account",
            "vos_import_rates",
        ] {
            assert_eq!(
                tool_policy(name).map(|item| item.risk),
                Some(ToolRisk::HighRisk)
            );
        }
    }

    #[test]
    fn query_and_export_tools_are_read_only() {
        for name in [
            "vos_get_dashboard_stats",
            "vos_list_cdrs",
            "vos_export_rates",
        ] {
            assert_eq!(
                tool_policy(name).map(|item| item.risk),
                Some(ToolRisk::ReadOnly)
            );
        }
    }

    #[test]
    fn tool_permissions_reuse_http_permission_catalog() {
        let approval_path = "/api/v1/copilot/sessions/cp-1/actions/cpa-1/approve";
        assert_eq!(
            required_permission("POST", approval_path),
            Some("copilot.execute")
        );

        let recharge = tool_policy("vos_recharge_billing_account").expect("known tool");
        assert_eq!(
            required_permission(recharge.method, recharge.path),
            Some("billing.accounts.credit")
        );
        let route = tool_policy("vos_create_route").expect("known tool");
        assert_eq!(
            required_permission(route.method, route.path),
            Some("routing.create")
        );
        let billing = tool_policy("vos_list_billing_accounts").expect("known tool");
        assert_eq!(
            required_permission(billing.method, billing.path),
            Some("billing.accounts.view")
        );
        let extensions = tool_policy("vos_list_extensions").expect("known tool");
        assert_eq!(
            required_permission(extensions.method, extensions.path),
            Some("extensions.view")
        );
    }

    #[test]
    fn unknown_tool_is_denied() {
        assert!(tool_policy("vos_run_shell").is_none());
    }

    #[test]
    fn explicit_failure_result_is_not_approved() {
        assert!(!tool_result_succeeded(&json!({
            "success": false,
            "warning": "目标配置冲突"
        })));
        assert!(!tool_result_succeeded(&json!({ "error": "执行失败" })));
        assert!(tool_result_succeeded(&json!({ "success": true })));
    }
}
