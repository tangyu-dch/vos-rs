//! 后台定时任务模块
//!
//! 包含会话定时器看门狗、NAT 保活、网关健康探测、订阅清理和 MOS 计算等功能。

mod gateway_health;
mod misc;
mod session;

pub(crate) use gateway_health::{
    persist_gateway_health, record_probe_failure, record_probe_success,
    spawn_gateway_health_probe_loop,
};
pub(crate) use misc::{
    calculate_mos_for_legs, spawn_nat_keepalive_loop, spawn_subscription_prune_loop,
};
pub(crate) use session::spawn_session_timer_watchdog;
