use super::worker::{spawn_recording_worker, RecordingCommand};
use super::{
    RecordingChannel, RecordingSessionInfo, RecordingWriterFactory, StandardWavWriterFactory,
};
use rtp_core::{PacketBufferPool, ReusablePacket};
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tracing::warn;

const RECORDING_PACKET_POOL_CAPACITY: usize = 4_096;

pub trait RecordingFinalizer: Send + Sync {
    fn finalize(&self, wav_path: PathBuf, format: String, call_id: String);
}

pub struct RecordingPool {
    workers: Vec<RecordingWorkerHandle>,
    queue_capacity: usize,
    packet_pool: PacketBufferPool,
    finalizer: Option<Arc<dyn RecordingFinalizer>>,
    writer_factory: Arc<dyn RecordingWriterFactory>,
}

impl std::fmt::Debug for RecordingPool {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RecordingPool")
            .field("workers", &self.workers)
            .field("queue_capacity", &self.queue_capacity)
            .field("packet_pool", &self.packet_pool)
            .field("finalizer", &self.finalizer.as_ref().map(|_| "configured"))
            .field("writer_header_bytes", &self.writer_factory.header_bytes())
            .finish()
    }
}

#[derive(Debug)]
struct RecordingWorkerHandle {
    sender: tokio::sync::mpsc::UnboundedSender<RecordingCommand>,
    pending_commands: Arc<AtomicUsize>,
}

impl RecordingPool {
    pub fn new(
        worker_count: usize,
        queue_capacity: usize,
        finalizer: Option<Arc<dyn RecordingFinalizer>>,
    ) -> Self {
        Self::with_writer_factory(
            worker_count,
            queue_capacity,
            finalizer,
            Arc::new(StandardWavWriterFactory),
        )
    }

    pub fn with_writer_factory(
        worker_count: usize,
        queue_capacity: usize,
        finalizer: Option<Arc<dyn RecordingFinalizer>>,
        writer_factory: Arc<dyn RecordingWriterFactory>,
    ) -> Self {
        let worker_count = worker_count.max(1);
        let queue_capacity = queue_capacity.max(2);
        let mut workers = Vec::with_capacity(worker_count);
        for worker_index in 0..worker_count {
            let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
            let pending_commands = Arc::new(AtomicUsize::new(0));
            match spawn_recording_worker(
                worker_index,
                receiver,
                Arc::clone(&pending_commands),
                finalizer.clone(),
                Arc::clone(&writer_factory),
            ) {
                Ok(()) => workers.push(RecordingWorkerHandle {
                    sender,
                    pending_commands,
                }),
                Err(error) => warn!(%error, worker_index, "failed to spawn recording worker"),
            }
        }
        Self {
            workers,
            queue_capacity,
            packet_pool: PacketBufferPool::new(RECORDING_PACKET_POOL_CAPACITY),
            finalizer,
            writer_factory,
        }
    }

    pub fn header_bytes(&self) -> u64 {
        self.writer_factory.header_bytes()
    }

    pub fn worker_count(&self) -> usize {
        self.workers.len()
    }

    pub fn queued_commands(&self) -> usize {
        self.workers
            .iter()
            .map(|worker| worker.pending_commands.load(Ordering::Relaxed))
            .sum()
    }

    pub fn total_capacity(&self) -> usize {
        self.workers.len() * self.queue_capacity
    }

    pub fn try_record(
        &self,
        session: Arc<RecordingSessionInfo>,
        packet: RecordedRtpPacket,
    ) -> io::Result<bool> {
        self.try_send_packet(session.id, RecordingCommand::Packet { session, packet })?;
        Ok(true)
    }

    pub(crate) fn copy_payload(&self, payload: &[u8]) -> ReusablePacket {
        self.packet_pool.copy(payload)
    }

    #[doc(hidden)]
    pub fn flush(&self, session: Arc<RecordingSessionInfo>) -> io::Result<()> {
        let (reply, receiver) = std::sync::mpsc::channel();
        self.send(session.id, RecordingCommand::Flush { session, reply })?;
        receiver
            .recv()
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "recording worker is stopped"))?
    }

    pub fn finish(&self, session_id: u64) {
        if let Err(error) = self.send(session_id, RecordingCommand::Finish { session_id }) {
            warn!(%error, session_id, "failed to enqueue recording finalization");
        }
    }

    fn try_send_packet(&self, session_id: u64, command: RecordingCommand) -> io::Result<()> {
        let worker = self.worker(session_id)?;
        let packet_limit = self.queue_capacity.saturating_sub(1);
        let mut pending = worker.pending_commands.load(Ordering::Acquire);
        loop {
            if pending >= packet_limit {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "recording queue reserved for control command",
                ));
            }
            match worker.pending_commands.compare_exchange(
                pending,
                pending + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(current) => pending = current,
            }
        }

        match worker.sender.send(command) {
            Ok(()) => Ok(()),
            Err(_) => {
                worker.pending_commands.fetch_sub(1, Ordering::AcqRel);
                Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "recording worker is stopped",
                ))
            }
        }
    }

    fn send(&self, session_id: u64, command: RecordingCommand) -> io::Result<()> {
        let worker = self.worker(session_id)?;
        worker.pending_commands.fetch_add(1, Ordering::AcqRel);
        worker.sender.send(command).map_err(|_| {
            worker.pending_commands.fetch_sub(1, Ordering::AcqRel);
            io::Error::new(io::ErrorKind::BrokenPipe, "recording worker is stopped")
        })
    }

    fn worker(&self, session_id: u64) -> io::Result<&RecordingWorkerHandle> {
        if self.workers.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "recording worker pool is unavailable",
            ));
        }
        Ok(&self.workers[session_id as usize % self.workers.len()])
    }
}

pub struct RecordedRtpPacket {
    pub channel: RecordingChannel,
    pub payload_type: u8,
    pub timestamp: u32,
    pub payload: ReusablePacket,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recording::{RecordingChannel, RecordingSession};
    use rtp_core::{RtpPacket, RtpPacketView};
    use std::sync::mpsc;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    struct CapturingFinalizer(mpsc::Sender<PathBuf>);

    impl RecordingFinalizer for CapturingFinalizer {
        fn finalize(&self, wav_path: PathBuf, _format: String, _call_id: String) {
            let _ = self.0.send(wav_path);
        }
    }

    #[test]
    fn rotation_finalizes_every_segment_with_its_actual_path() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("media-core-rotation-{suffix}"));
        std::fs::create_dir_all(&directory).unwrap();
        let wav_path = directory.join("call.wav");
        let (sender, receiver) = mpsc::channel();
        let finalizer: Arc<dyn RecordingFinalizer> = Arc::new(CapturingFinalizer(sender));
        let pool = Arc::new(RecordingPool::new(1, 8, Some(finalizer)));
        let session = RecordingSession::new(
            "call-1".to_string(),
            wav_path.clone(),
            0,
            48,
            0,
            pool,
            "wav".to_string(),
        );

        for (sequence, timestamp) in [(1, 0), (2, 2)] {
            let encoded = RtpPacket::new(0, sequence, timestamp, 7, vec![0xff, 0xff])
                .unwrap()
                .encode()
                .unwrap();
            let packet = RtpPacketView::parse(&encoded).unwrap();
            session
                .try_record(RecordingChannel::Caller, packet)
                .unwrap();
            session.flush().unwrap();
        }
        drop(session);

        let first = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        let second = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        let part_path = directory.join("call-part-0001.wav");
        assert_eq!([first, second], [wav_path.clone(), part_path.clone()]);
        assert!(wav_path.with_extension("json").exists());
        assert!(part_path.with_extension("json").exists());

        std::fs::remove_dir_all(directory).unwrap();
    }
}
