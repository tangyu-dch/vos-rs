use std::{env, io, str::FromStr};

use call_core::{Route, RouteTable, RouteTarget};
use cdr_core::{PostgresCdrStore, DEFAULT_CDR_STREAM, DEFAULT_CDR_SUBJECT};
use sip_core::SipUri;
use tracing::{debug, info};

use crate::config::{
    AnyError, DATABASE_URL_ENV, DEFAULT_GATEWAY_ENV, NATS_CDR_STREAM_ENV,
    NATS_CDR_SUBJECT_ENV, NATS_URL_ENV,
};
use crate::nats_cdr::NatsCdrPublisher;
use crate::EdgeState;

#[derive(Debug, Clone, Default)]
pub struct CdrSinks {
    pub postgres: Option<PostgresCdrStore>,
    pub nats: Option<NatsCdrPublisher>,
}

pub async fn cdr_sinks_from_env() -> Result<CdrSinks, AnyError> {
    let nats = match env::var(NATS_URL_ENV) {
        Ok(nats_url) => {
            let subject =
                env::var(NATS_CDR_SUBJECT_ENV).unwrap_or_else(|_| DEFAULT_CDR_SUBJECT.to_string());
            let stream =
                env::var(NATS_CDR_STREAM_ENV).unwrap_or_else(|_| DEFAULT_CDR_STREAM.to_string());
            let publisher =
                NatsCdrPublisher::connect(&nats_url, subject.clone(), stream.clone()).await?;
            info!(subject, stream, "NATS JetStream CDR queue enabled");
            Some(publisher)
        }
        Err(_) => {
            info!(
                env = NATS_URL_ENV,
                "NATS CDR queue disabled; set env var to enable"
            );
            None
        }
    };

    let postgres = match env::var(DATABASE_URL_ENV) {
        Ok(database_url) => {
            let store = PostgresCdrStore::connect(&database_url).await?;
            if nats.is_some() {
                info!("PostgreSQL direct CDR persistence disabled because NATS CDR queue is enabled (database connection will still be used for configuration and registration store)");
            } else {
                info!("PostgreSQL CDR persistence enabled");
            }
            Some(store)
        }
        Err(_) => {
            info!(
                env = DATABASE_URL_ENV,
                "PostgreSQL database connection disabled; set env var to enable"
            );
            None
        }
    };

    Ok(CdrSinks { postgres, nats })
}

pub async fn flush_completed_cdrs(
    cdr_sinks: &CdrSinks,
    edge_state: &EdgeState,
) -> Result<(), AnyError> {
    let cdrs = edge_state.call_manager.completed_cdrs().to_vec();

    if cdrs.is_empty() {
        return Ok(());
    }

    if let Some(nats) = &cdr_sinks.nats {
        for cdr in &cdrs {
            nats.publish_cdr(cdr).await?;
        }

        let queued = edge_state.call_manager.take_completed_cdrs().len();
        debug!(count = queued, "queued completed CDRs to NATS");
        return Ok(());
    }

    if let Some(cdr_store) = &cdr_sinks.postgres {
        for cdr in &cdrs {
            cdr_store.insert_call_cdr(cdr).await?;
        }

        let persisted = edge_state.call_manager.take_completed_cdrs().len();
        debug!(count = persisted, "persisted completed CDRs to PostgreSQL");
        return Ok(());
    }

    {
        let dropped = edge_state.call_manager.take_completed_cdrs().len();
        debug!(count = dropped, "discarded completed CDRs without CDR sink");
    }
    Ok(())
}

pub fn route_table_from_env() -> Result<RouteTable, AnyError> {
    let Ok(gateway) = env::var(DEFAULT_GATEWAY_ENV) else {
        return Ok(RouteTable::default());
    };

    let target = parse_gateway_target("default", &gateway)?;
    Ok(RouteTable::new(vec![Route::new(
        "default", "", 100, target,
    )]))
}

pub fn parse_gateway_target(gateway_id: &str, raw: &str) -> Result<RouteTarget, AnyError> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(Box::new(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{DEFAULT_GATEWAY_ENV} must not be empty"),
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
