use super::{
    write_recording_segment_metadata, RecordedRtpPacket, RecordingFile, RecordingFinalizer,
    RecordingSessionInfo, RecordingWriterFactory, RECORDING_WORKER_DRAIN_LIMIT,
};
use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use tracing::warn;

pub enum RecordingCommand {
    Packet {
        session: Arc<RecordingSessionInfo>,
        packet: RecordedRtpPacket,
    },
    Flush {
        session: Arc<RecordingSessionInfo>,
        reply: std::sync::mpsc::Sender<io::Result<()>>,
    },
    Finish {
        session_id: u64,
    },
}

pub(super) fn spawn_recording_worker(
    worker_index: usize,
    receiver: tokio::sync::mpsc::UnboundedReceiver<RecordingCommand>,
    pending_commands: Arc<AtomicUsize>,
    finalizer: Option<Arc<dyn RecordingFinalizer>>,
    writer_factory: Arc<dyn RecordingWriterFactory>,
) -> io::Result<()> {
    thread::Builder::new()
        .name(format!("vos-rs-recording-{worker_index}"))
        .spawn(move || {
            run_recording_worker(
                worker_index,
                receiver,
                pending_commands,
                finalizer,
                writer_factory,
            )
        })
        .map(|_| ())
        .map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("failed to spawn recording worker: {error}"),
            )
        })
}

fn run_recording_worker(
    worker_index: usize,
    mut receiver: tokio::sync::mpsc::UnboundedReceiver<RecordingCommand>,
    pending_commands: Arc<AtomicUsize>,
    finalizer: Option<Arc<dyn RecordingFinalizer>>,
    writer_factory: Arc<dyn RecordingWriterFactory>,
) {
    let mut recorders = HashMap::<u64, RecordingFile>::new();
    while let Some(command) = receiver.blocking_recv() {
        pending_commands.fetch_sub(1, Ordering::Relaxed);
        handle_recording_command(
            command,
            &mut recorders,
            finalizer.as_ref(),
            writer_factory.as_ref(),
        );

        for _ in 0..RECORDING_WORKER_DRAIN_LIMIT {
            let Ok(command) = receiver.try_recv() else {
                break;
            };
            pending_commands.fetch_sub(1, Ordering::Relaxed);
            handle_recording_command(
                command,
                &mut recorders,
                finalizer.as_ref(),
                writer_factory.as_ref(),
            );
        }
    }

    for (session_id, recording_file) in recorders {
        if let Err(error) = finalize_file(recording_file, finalizer.as_ref()) {
            warn!(%error, session_id, worker_index, "failed to finalize recording during worker shutdown");
        }
    }
}

fn handle_recording_command(
    command: RecordingCommand,
    recorders: &mut HashMap<u64, RecordingFile>,
    finalizer: Option<&Arc<dyn RecordingFinalizer>>,
    writer_factory: &dyn RecordingWriterFactory,
) {
    match command {
        RecordingCommand::Packet { session, packet } => {
            handle_packet(recorders, session, packet, finalizer, writer_factory)
        }
        RecordingCommand::Flush { session, reply } => {
            let result = recorder_for_session(recorders, &session, writer_factory)
                .and_then(|recording_file| recording_file.recorder.flush_recording());
            let _ = reply.send(result);
        }
        RecordingCommand::Finish { session_id } => {
            match finalize_recording(recorders, session_id, finalizer) {
                Ok(()) => {}
                Err(error) => {
                    warn!(%error, session_id, "failed to finalize call recording");
                }
            }
        }
    }
}

fn handle_packet(
    recorders: &mut HashMap<u64, RecordingFile>,
    session: Arc<RecordingSessionInfo>,
    packet: RecordedRtpPacket,
    finalizer: Option<&Arc<dyn RecordingFinalizer>>,
    writer_factory: &dyn RecordingWriterFactory,
) {
    if let Err(error) = session.ensure_disk_space() {
        warn!(%error, session_id = session.id, "recording disk protection stopped packet write");
        return;
    }
    let should_rotate = recorders.get(&session.id).is_some_and(|recording_file| {
        recording_file.recorder.would_exceed_limit(
            packet.channel,
            packet.timestamp,
            packet.payload.as_slice().len(),
            session.max_file_frames(),
        )
    });
    if should_rotate {
        if let Err(error) = rotate_recording(recorders, &session, finalizer, writer_factory) {
            warn!(%error, session_id = session.id, "failed to rotate call recording");
            return;
        }
    }
    let recording_file = match recorder_for_session(recorders, &session, writer_factory) {
        Ok(recording_file) => recording_file,
        Err(error) => {
            warn!(%error, session_id = session.id, "failed to open call recording");
            return;
        }
    };
    if let Err(error) = recording_file.recorder.record(
        packet.channel,
        packet.payload_type,
        packet.timestamp,
        packet.payload.as_slice(),
    ) {
        warn!(%error, session_id = session.id, "failed to write RTP packet to recording");
    }
}

fn finalize_recording(
    recorders: &mut HashMap<u64, RecordingFile>,
    session_id: u64,
    finalizer: Option<&Arc<dyn RecordingFinalizer>>,
) -> io::Result<()> {
    let Some(recording_file) = recorders.remove(&session_id) else {
        return Ok(());
    };
    finalize_file(recording_file, finalizer)
}

fn recorder_for_session<'a>(
    recorders: &'a mut HashMap<u64, RecordingFile>,
    session: &Arc<RecordingSessionInfo>,
    writer_factory: &dyn RecordingWriterFactory,
) -> io::Result<&'a mut RecordingFile> {
    if let std::collections::hash_map::Entry::Vacant(entry) = recorders.entry(session.id) {
        entry.insert(RecordingFile::create(session, 0, writer_factory)?);
    }
    recorders
        .get_mut(&session.id)
        .ok_or_else(|| io::Error::other("recording session was not initialized"))
}

fn rotate_recording(
    recorders: &mut HashMap<u64, RecordingFile>,
    session: &Arc<RecordingSessionInfo>,
    finalizer: Option<&Arc<dyn RecordingFinalizer>>,
    writer_factory: &dyn RecordingWriterFactory,
) -> io::Result<()> {
    let segment_index = recorders
        .get(&session.id)
        .map(|recording_file| recording_file.segment_index + 1)
        .unwrap_or(1);
    if let Some(previous) = recorders.remove(&session.id) {
        finalize_file(previous, finalizer)?;
    }
    recorders.insert(
        session.id,
        RecordingFile::create(session, segment_index, writer_factory)?,
    );
    Ok(())
}

fn finalize_file(
    mut recording_file: RecordingFile,
    finalizer: Option<&Arc<dyn RecordingFinalizer>>,
) -> io::Result<()> {
    recording_file.recorder.flush_recording()?;
    write_recording_segment_metadata(
        &recording_file.session,
        recording_file.segment_index,
        recording_file.recorder.flushed_frames(),
    )?;
    if let Some(finalizer) = finalizer {
        finalizer.finalize(
            recording_file.path,
            recording_file.session.format_str.clone(),
            recording_file.session.call_id.clone(),
        );
    }
    Ok(())
}
