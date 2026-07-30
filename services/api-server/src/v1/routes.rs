use axum::{
    routing::{get, post, put},
    Router,
};

use crate::{
    access_control, announcements,
    billing::{accounts, anti_fraud, cdr, operations, report},
    cluster::{calls, media_cluster, sip_cluster},
    copilot::history as copilot_history,
    copilot::safety as copilot_safety,
    copilot::stream as copilot_stream,
    dashboard, details, llm_configs, notifications, recording,
    resources::{
        call_center, gateways, ivr_menus, numbers, prompts, registrations, routes, tenants, users,
    },
    system::{audit, system},
    termination, AppState,
};

use super::handle_copilot_chat;

pub(super) fn access_control_routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/access-control", get(access_control::overview))
        .route(
            "/api/v1/access-control/accounts",
            get(access_control::overview),
        )
        .route(
            "/api/v1/access-control/role-permissions",
            get(access_control::overview),
        )
        .route(
            "/api/v1/access-control/users",
            post(access_control::create_user),
        )
        .route(
            "/api/v1/access-control/users/:username",
            put(access_control::update_user).delete(access_control::delete_user),
        )
        .route(
            "/api/v1/access-control/roles",
            post(access_control::create_role),
        )
        .route(
            "/api/v1/access-control/roles/:role_key",
            put(access_control::update_role).delete(access_control::delete_role),
        )
        .route(
            "/api/v1/access-control/roles/user-assignments",
            put(access_control::assign_user_roles),
        )
        .route(
            "/api/v1/access-control/roles/:role_key/permissions",
            put(access_control::replace_permissions),
        )
        .route(
            "/api/v1/access-control/menus/:item_key",
            put(access_control::update_menu),
        )
}

pub(super) fn notification_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/notifications",
            get(notifications::list_notifications),
        )
        .route(
            "/api/v1/notifications/unread-count",
            get(notifications::unread_count),
        )
        .route(
            "/api/v1/notifications/read-all",
            post(notifications::mark_all_read),
        )
        .route("/api/v1/notifications/scan", post(notifications::scan_now))
        .route(
            "/api/v1/notifications/:id/read",
            post(notifications::mark_read),
        )
}

pub(super) fn announcement_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/announcements",
            get(announcements::list_announcements).post(announcements::create_announcement),
        )
        .route(
            "/api/v1/announcements/:id/publish",
            post(announcements::publish_announcement),
        )
        .route(
            "/api/v1/announcements/:id",
            get(announcements::get_announcement)
                .put(announcements::update_announcement)
                .delete(announcements::delete_announcement),
        )
        .route(
            "/api/v1/my-announcements",
            get(announcements::list_my_announcements),
        )
        .route(
            "/api/v1/my-announcements/:id/read",
            post(announcements::mark_my_announcement_read),
        )
        .route(
            "/api/v1/my-announcements/:id",
            get(announcements::get_my_announcement),
        )
}

pub(super) fn overview_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/overview/summary",
            get(dashboard::get_dashboard_stats),
        )
        .route(
            "/api/v1/overview/trends",
            get(dashboard::get_dashboard_trend),
        )
        .route(
            "/api/v1/overview/node-traffic",
            get(dashboard::get_node_traffic),
        )
        .route(
            "/api/v1/overview/monitoring-extras",
            get(dashboard::get_monitoring_extras),
        )
        .route("/api/v1/overview/events", get(dashboard::dashboard_events))
}

pub(super) fn subscriber_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/extensions",
            get(users::list_users).post(users::create_user),
        )
        .route(
            "/api/v1/extensions/import",
            post(crate::import::import_users),
        )
        .route(
            "/api/v1/extensions/import-template",
            get(crate::import::import_users_template),
        )
        .route(
            "/api/v1/extensions/:username",
            get(details::extension)
                .put(users::update_user)
                .delete(users::delete_user),
        )
        .route(
            "/api/v1/registrations",
            get(registrations::list_registrations),
        )
        .route(
            "/api/v1/numbers",
            get(numbers::list_numbers).post(numbers::create_number),
        )
        .route(
            "/api/v1/numbers/import",
            post(crate::import::import_numbers),
        )
        .route(
            "/api/v1/numbers/import-template",
            get(crate::import::import_numbers_template),
        )
        .route(
            "/api/v1/numbers/:number",
            put(numbers::update_number).delete(numbers::delete_number),
        )
}

pub(super) fn interconnect_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/trunks",
            get(gateways::list_gateways).post(gateways::create_gateway),
        )
        .route(
            "/api/v1/trunks/:id",
            get(details::trunk)
                .put(gateways::update_gateway)
                .delete(gateways::delete_gateway),
        )
}

pub(super) fn termination_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/trunks/:id/ip-rules",
            get(termination::list_ip_rules).put(termination::replace_ip_rules),
        )
        .route(
            "/api/v1/trunks/:id/egress-endpoints",
            get(termination::list_endpoints).put(termination::replace_endpoints),
        )
        .route(
            "/api/v1/trunks/:id/outbound-policy",
            get(termination::get_trunk_policy).put(termination::put_trunk_policy),
        )
        .route(
            "/api/v1/extensions/:username/outbound-policy",
            get(termination::get_extension_policy).put(termination::put_extension_policy),
        )
        .route(
            "/api/v1/numbers/:number/owner",
            put(termination::set_number_owner),
        )
        .route(
            "/api/v1/numbers/:number/allocations",
            get(termination::list_allocations).put(termination::replace_allocations),
        )
        .route(
            "/api/v1/numbers/:number/did-destination",
            get(termination::get_number_did).put(termination::put_number_did),
        )
        .route(
            "/api/v1/caller-pools",
            get(termination::list_caller_pools).post(termination::create_caller_pool),
        )
        .route(
            "/api/v1/caller-pools/:id",
            put(termination::update_caller_pool).delete(termination::delete_caller_pool),
        )
        .route(
            "/api/v1/caller-pools/:id/members",
            get(termination::list_caller_pool_members)
                .put(termination::replace_caller_pool_members),
        )
        .route(
            "/api/v1/egress-groups",
            get(termination::list_egress_groups).post(termination::create_egress_group),
        )
        .route(
            "/api/v1/egress-groups/:id",
            put(termination::update_egress_group).delete(termination::delete_egress_group),
        )
        .route(
            "/api/v1/egress-groups/:id/members",
            get(termination::list_egress_group_members)
                .put(termination::replace_egress_group_members),
        )
        .route("/api/v1/outbound-policies", get(termination::list_policies))
        .route(
            "/api/v1/outbound-policies/:source_type/:source_id",
            get(termination::get_policy)
                .put(termination::put_policy)
                .delete(termination::delete_policy),
        )
        .route(
            "/api/v1/did-destinations",
            get(termination::list_dids).post(termination::create_did),
        )
        .route(
            "/api/v1/did-destinations/:number",
            put(termination::update_did).delete(termination::delete_did),
        )
}

pub(super) fn routing_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/routing/rules",
            get(routes::list_routes).post(routes::create_route),
        )
        .route(
            "/api/v1/routing/rules/import",
            post(crate::import::import_routes),
        )
        .route(
            "/api/v1/routing/rules/import-template",
            get(crate::import::import_routes_template),
        )
        .route(
            "/api/v1/routing/rules/:id",
            put(routes::update_route).delete(routes::delete_route),
        )
        .route("/api/v1/routing/simulations", get(calls::route_preview))
}

pub(super) fn call_routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/calls", get(cdr::list_cdrs))
        .route("/api/v1/calls/active", get(calls::list_active))
        .route("/api/v1/calls/:call_id", get(calls::call_detail))
        .route("/api/v1/calls/:call_id/media", get(calls::call_media))
        .route("/api/v1/calls/:call_id/dtmf", get(cdr::get_dtmf_events))
        .route("/api/v1/calls/:call_id/sipflow", get(calls::call_sipflow))
        .route(
            "/api/v1/calls/:call_id/recording",
            get(recording::get_recording_audio),
        )
        .route(
            "/api/v1/calls/:call_id/actions/terminate",
            post(calls::terminate_call),
        )
        .route("/api/v1/calls/:call_id/actions/play", post(calls::play))
        .route(
            "/api/v1/calls/:call_id/actions/stop-play",
            post(calls::stop_play),
        )
        .route("/api/v1/calls/:call_id/actions/mute", post(calls::mute))
        .route("/api/v1/calls/:call_id/actions/unmute", post(calls::unmute))
        .route(
            "/api/v1/calls/:call_id/actions/monitor",
            post(calls::monitor),
        )
        .route(
            "/api/v1/calls/:call_id/actions/stop-monitor",
            post(calls::stop_monitor),
        )
        .route("/api/v1/reports/summary", get(report::get_report_summary))
        .route("/api/v1/reports/export", get(report::export_cdrs_csv))
}

pub(super) fn billing_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/billing/access-accounts",
            get(accounts::list_access_accounts).post(accounts::create_access_account),
        )
        .route(
            "/api/v1/billing/access-accounts/:id",
            put(accounts::update_access_account).delete(accounts::delete_access_account),
        )
        .route(
            "/api/v1/billing/access-accounts/:id/credit",
            post(accounts::credit_access_account),
        )
        .route(
            "/api/v1/billing/egress-accounts",
            get(accounts::list_egress_accounts).post(accounts::create_egress_account),
        )
        .route(
            "/api/v1/billing/egress-accounts/:id",
            put(accounts::update_egress_account).delete(accounts::delete_egress_account),
        )
        .route(
            "/api/v1/billing/egress-accounts/:id/credit",
            post(accounts::credit_egress_account),
        )
        .route("/api/v1/billing/credits", get(accounts::list_journal))
        .route("/api/v1/billing/transactions", get(operations::list_ledger))
}

pub(super) fn security_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/security/anti-fraud/policies",
            get(anti_fraud::list_anti_fraud_rules).post(anti_fraud::create_anti_fraud_rule),
        )
        .route(
            "/api/v1/security/anti-fraud/policies/:id",
            put(anti_fraud::update_anti_fraud_rule).delete(anti_fraud::delete_anti_fraud_rule),
        )
        .route(
            "/api/v1/security/anti-fraud/settings",
            get(anti_fraud::list_anti_fraud_config),
        )
        .route(
            "/api/v1/security/anti-fraud/settings/:key",
            put(anti_fraud::update_anti_fraud_config),
        )
        .route(
            "/api/v1/security/anti-fraud/deepfake-logs",
            get(anti_fraud::get_deepfake_logs),
        )
        .route("/api/v1/security/audit-logs", get(audit::list_audit_logs))
        .route("/api/v1/copilot/chat", post(handle_copilot_chat))
        // Copilot 历史会话：行级按 operator 隔离，所有写操作要求 Bearer JWT
        .route(
            "/api/v1/copilot/sessions",
            get(copilot_history::list_sessions).post(copilot_history::create_session),
        )
        .route(
            "/api/v1/copilot/sessions/:id",
            get(copilot_history::get_session)
                .put(copilot_history::update_session)
                .delete(copilot_history::delete_session),
        )
        .route(
            "/api/v1/copilot/sessions/:id/chat",
            post(copilot_history::chat_in_session),
        )
        .route(
            "/api/v1/copilot/sessions/:id/chat/stream",
            post(copilot_stream::chat_in_session_stream),
        )
        .route(
            "/api/v1/copilot/sessions/:id/actions",
            get(copilot_safety::list_actions),
        )
        .route(
            "/api/v1/copilot/sessions/:id/actions/:action_id/approve",
            post(copilot_safety::approve_action),
        )
        .route(
            "/api/v1/copilot/sessions/:id/actions/:action_id/reject",
            post(copilot_safety::reject_action),
        )
        // LLM 配置管理：多厂商配置 CRUD + 启用切换，Copilot 运行时动态读取
        .route(
            "/api/v1/llm-configs",
            get(llm_configs::list_llm_configs).post(llm_configs::create_llm_config),
        )
        .route(
            "/api/v1/llm-configs/active",
            get(llm_configs::get_active_llm_config),
        )
        .route(
            "/api/v1/llm-configs/:id",
            put(llm_configs::update_llm_config).delete(llm_configs::delete_llm_config),
        )
        .route(
            "/api/v1/llm-configs/:id/activate",
            post(llm_configs::activate_llm_config),
        )
}

pub(super) fn infrastructure_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/infrastructure/settings",
            get(system::get_system_configs).post(system::update_system_configs),
        )
        .route(
            "/api/v1/infrastructure/media-cluster",
            get(media_cluster::get_media_cluster).put(media_cluster::update_media_cluster),
        )
        .route(
            "/api/v1/infrastructure/sip-cluster",
            get(sip_cluster::get_sip_cluster_status),
        )
        .route(
            "/api/v1/infrastructure/sip-cluster/nodes/:node_id/:action",
            post(sip_cluster::control_sip_cluster_node),
        )
        .route(
            "/api/v1/infrastructure/media/metrics",
            get(calls::media_metrics),
        )
}

pub(super) fn call_center_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/call-center/queues",
            get(call_center::list_queues).post(call_center::create_queue),
        )
        .route(
            "/api/v1/call-center/queues/:id",
            put(call_center::update_queue).delete(call_center::delete_queue),
        )
        .route(
            "/api/v1/call-center/agents",
            get(call_center::list_agents).post(call_center::create_agent),
        )
        .route(
            "/api/v1/call-center/agents/:id",
            put(call_center::update_agent).delete(call_center::delete_agent),
        )
}

pub(super) fn ivr_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/ivr/menus",
            get(ivr_menus::list_menus).post(ivr_menus::create_menu),
        )
        .route(
            "/api/v1/ivr/menus/:id",
            get(ivr_menus::get_menu)
                .put(ivr_menus::update_menu)
                .delete(ivr_menus::delete_menu),
        )
        .route("/api/v1/ivr/prompts", get(prompts::list_prompts))
        .route("/api/v1/ivr/prompts/upload", post(prompts::upload_prompt))
        .route(
            "/api/v1/ivr/prompts/:filename",
            get(prompts::get_prompt).delete(prompts::delete_prompt),
        )
}

pub(super) fn tenant_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/tenants",
            get(tenants::list_tenants).post(tenants::create_tenant),
        )
        .route(
            "/api/v1/tenants/:id",
            get(tenants::get_tenant)
                .put(tenants::update_tenant)
                .delete(tenants::delete_tenant),
        )
        .route(
            "/api/v1/tenants/:id/enabled",
            post(tenants::toggle_tenant_enabled),
        )
}
