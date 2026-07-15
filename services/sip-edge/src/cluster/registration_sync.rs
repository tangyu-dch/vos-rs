//! REGISTER 状态到 Redis 的有界批量同步。

use std::time::Duration;

const REGISTRATION_SYNC_QUEUE_CAPACITY: usize = 4_096;
const REGISTRATION_SYNC_BATCH_SIZE: usize = 128;
const REGISTRATION_SYNC_MAX_ATTEMPTS: u32 = 3;

#[derive(Debug)]
pub(crate) enum RegistrationSyncCommand {
    Upsert {
        registration_key: String,
        contacts_json: String,
        flow: Option<(String, String)>,
        ttl_secs: u64,
    },
    Delete {
        registration_key: String,
        flow_key: String,
    },
}

#[derive(Clone)]
pub(crate) struct RegistrationSyncSender {
    sender: tokio::sync::mpsc::Sender<RegistrationSyncCommand>,
}

impl RegistrationSyncSender {
    /// 将注册状态放入有界队列；队列饱和时施加背压，避免无限创建任务。
    pub(crate) async fn send(
        &self,
        command: RegistrationSyncCommand,
    ) -> Result<(), tokio::sync::mpsc::error::SendError<RegistrationSyncCommand>> {
        self.sender.send(command).await
    }
}

pub(crate) fn start_registration_sync(
    redis: redis::aio::ConnectionManager,
) -> RegistrationSyncSender {
    let (sender, receiver) = tokio::sync::mpsc::channel(REGISTRATION_SYNC_QUEUE_CAPACITY);
    tokio::spawn(run_registration_sync(redis, receiver));
    RegistrationSyncSender { sender }
}

async fn run_registration_sync(
    mut redis: redis::aio::ConnectionManager,
    mut receiver: tokio::sync::mpsc::Receiver<RegistrationSyncCommand>,
) {
    while let Some(first) = receiver.recv().await {
        let batch = drain_batch(first, &mut receiver);
        let mut pipeline = redis::pipe();
        for command in batch {
            append_command(&mut pipeline, command);
        }
        let mut delivered = false;
        for attempt in 1..=REGISTRATION_SYNC_MAX_ATTEMPTS {
            match pipeline.query_async::<()>(&mut redis).await {
                Ok(()) => {
                    delivered = true;
                    break;
                }
                Err(error) if attempt < REGISTRATION_SYNC_MAX_ATTEMPTS => {
                    tracing::warn!(%error, attempt, "REGISTER Redis 批量同步失败，准备重试");
                    tokio::time::sleep(Duration::from_millis(25 * u64::from(attempt))).await;
                }
                Err(error) => tracing::error!(%error, attempt, "REGISTER Redis 批量同步最终失败"),
            }
        }
        if !delivered {
            tracing::error!("REGISTER Redis 同步批次已丢弃，等待终端重新注册恢复状态");
        }
    }
}

fn drain_batch(
    first: RegistrationSyncCommand,
    receiver: &mut tokio::sync::mpsc::Receiver<RegistrationSyncCommand>,
) -> Vec<RegistrationSyncCommand> {
    let mut batch = Vec::with_capacity(REGISTRATION_SYNC_BATCH_SIZE);
    batch.push(first);
    while batch.len() < REGISTRATION_SYNC_BATCH_SIZE {
        match receiver.try_recv() {
            Ok(command) => batch.push(command),
            Err(_) => break,
        }
    }
    batch
}

fn append_command(pipeline: &mut redis::Pipeline, command: RegistrationSyncCommand) {
    match command {
        RegistrationSyncCommand::Upsert {
            registration_key,
            contacts_json,
            flow,
            ttl_secs,
        } => {
            pipeline
                .cmd("SET")
                .arg(registration_key)
                .arg(contacts_json)
                .arg("EX")
                .arg(ttl_secs)
                .ignore();
            if let Some((flow_key, flow_json)) = flow {
                pipeline
                    .cmd("SET")
                    .arg(flow_key)
                    .arg(flow_json)
                    .arg("EX")
                    .arg(ttl_secs)
                    .ignore();
            }
        }
        RegistrationSyncCommand::Delete {
            registration_key,
            flow_key,
        } => {
            pipeline
                .del(registration_key)
                .ignore()
                .del(flow_key)
                .ignore();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_drain_batch_collects_available_commands_without_waiting() {
        let (sender, mut receiver) = tokio::sync::mpsc::channel(4);
        for index in 0..3 {
            sender
                .send(RegistrationSyncCommand::Delete {
                    registration_key: format!("registration-{index}"),
                    flow_key: format!("flow-{index}"),
                })
                .await
                .expect("queue should accept command");
        }
        let first = receiver.recv().await.expect("first command");
        let batch = drain_batch(first, &mut receiver);
        assert_eq!(batch.len(), 3);
    }
}
