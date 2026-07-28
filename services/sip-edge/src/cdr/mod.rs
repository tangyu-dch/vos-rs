//! CDR 管线模块：批量写入、Spool 持久化与 NATS 发布。

mod nats_sink;
mod pipeline;
mod spool;

pub(crate) use nats_sink::NatsCdrPublisher;
pub(crate) use pipeline::{cdr_sinks_from_config, flush_cdr_batch_with_retry_and_spool};

#[cfg(test)]
pub(crate) use pipeline::flush_cdr_batch;
pub(crate) use spool::{
    configured_spool_dir, spawn_replay_loop, CdrPipelineMetrics, CdrPipelineSnapshot, CdrSpool,
    DurableCdrSink,
};
