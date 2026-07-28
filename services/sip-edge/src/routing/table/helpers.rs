use crate::config::EdgeConfig;
use call_core::{Route, RouteTable, RouteTarget};
use sip_core::SipUri;
use std::io;
use std::str::FromStr;

use super::AnyError;

pub(super) fn now_hhmm_or_current() -> String {
    cdr_core::current_hhmm().unwrap_or_else(|| "00:00".to_string())
}

pub(super) fn route_time_is_active(
    now: Option<&str>,
    time_start: Option<&str>,
    time_end: Option<&str>,
) -> bool {
    let (Some(now), Some(start), Some(end)) = (now, time_start, time_end) else {
        return true;
    };

    if start <= end {
        now >= start && now <= end
    } else {
        // A window such as 22:00-06:00 crosses midnight.
        now >= start || now <= end
    }
}

pub(crate) fn route_table_from_config(config: &EdgeConfig) -> Result<RouteTable, AnyError> {
    if config.default_gateway.is_empty() {
        return Ok(RouteTable::default());
    }

    let target = parse_gateway_target("default", &config.default_gateway)?;
    Ok(RouteTable::new(vec![Route::new(
        "default", "", 100, target,
    )]))
}

pub(crate) fn parse_gateway_target(gateway_id: &str, raw: &str) -> Result<RouteTarget, AnyError> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(Box::new(io::Error::new(
            io::ErrorKind::InvalidInput,
            "sip_edge.routing.default_gateway must not be empty",
        )));
    }

    let uri = if value.starts_with("sip:") || value.starts_with("sips:") {
        SipUri::from_str(value)
    } else {
        SipUri::from_str(&format!("sip:{value}"))
    }
    .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error.to_string()))?;

    Ok(RouteTarget::new(gateway_id, uri.host, uri.port))
}

#[cfg(test)]
mod tests {
    use super::route_time_is_active;

    #[test]
    fn route_time_window_supports_same_day_and_overnight_ranges() {
        assert!(route_time_is_active(
            Some("12:00"),
            Some("09:00"),
            Some("18:00")
        ));
        assert!(!route_time_is_active(
            Some("08:59"),
            Some("09:00"),
            Some("18:00")
        ));
        assert!(route_time_is_active(
            Some("23:30"),
            Some("22:00"),
            Some("06:00")
        ));
        assert!(route_time_is_active(
            Some("05:30"),
            Some("22:00"),
            Some("06:00")
        ));
        assert!(!route_time_is_active(
            Some("12:00"),
            Some("22:00"),
            Some("06:00")
        ));
    }
}
