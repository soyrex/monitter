//! Bounded, single-writer transport for line-framed child stdin pipes.
//!
//! Provider adapters still own the protocol JSON. This module only owns the
//! byte pipe, queue bounds, flush acknowledgements, and cancellation priority.

use std::{
    collections::VecDeque,
    io::Write,
    sync::{mpsc, Arc, Condvar, Mutex},
    thread,
    time::Duration,
};

pub(crate) const MAX_FRAME_BYTES: usize = 1024 * 1024;
const NORMAL_QUEUE_CAPACITY: usize = 64;
const MAX_QUEUED_BYTES: usize = 2 * 1024 * 1024;
const PRIORITY_QUEUE_CAPACITY: usize = 4;

#[derive(Clone)]
pub(crate) struct OutboundWriter {
    shared: Arc<Shared>,
}

struct Shared {
    state: Mutex<State>,
    ready: Condvar,
}

struct State {
    normal: VecDeque<Frame>,
    priority: VecDeque<Frame>,
    queued_bytes: usize,
    closed: bool,
}

struct Frame {
    bytes: Vec<u8>,
    delivered: mpsc::SyncSender<Result<(), String>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Priority {
    Normal,
    Control,
}

impl OutboundWriter {
    /// Starts the sole writer for a child stdin pipe. Dropping senders closes
    /// the queue; the writer exits once it finishes any in-flight write.
    pub(crate) fn spawn<W>(mut writer: W) -> Self
    where
        W: Write + Send + 'static,
    {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                normal: VecDeque::new(),
                priority: VecDeque::new(),
                queued_bytes: 0,
                closed: false,
            }),
            ready: Condvar::new(),
        });
        let worker_shared = Arc::clone(&shared);
        thread::Builder::new()
            .name("monitter-stdin-writer".into())
            .spawn(move || loop {
                let frame = {
                    let Ok(state) = worker_shared.state.lock() else {
                        break;
                    };
                    let mut state = match worker_shared.ready.wait_while(state, |state| {
                        state.normal.is_empty() && state.priority.is_empty() && !state.closed
                    }) {
                        Ok(state) => state,
                        Err(_) => break,
                    };
                    if let Some(frame) = state
                        .priority
                        .pop_front()
                        .or_else(|| state.normal.pop_front())
                    {
                        state.queued_bytes = state.queued_bytes.saturating_sub(frame.bytes.len());
                        frame
                    } else if state.closed {
                        break;
                    } else {
                        continue;
                    }
                };
                let result = writer
                    .write_all(&frame.bytes)
                    .and_then(|_| writer.flush())
                    .map_err(|error| format!("Could not flush provider control frame: {error}"));
                let failed = result.is_err();
                let _ = frame.delivered.send(result);
                if failed {
                    let Ok(mut state) = worker_shared.state.lock() else {
                        break;
                    };
                    state.closed = true;
                    let mut pending_frames = state.priority.drain(..).collect::<Vec<_>>();
                    pending_frames.extend(state.normal.drain(..));
                    for pending in pending_frames {
                        let _ = pending
                            .delivered
                            .send(Err("Provider stdin writer has failed.".into()));
                    }
                    state.queued_bytes = 0;
                    break;
                }
            })
            .expect("could not start provider stdin writer");
        Self { shared }
    }

    /// Enqueue one complete newline-delimited frame and await actual flush.
    /// An error after enqueue is terminal for that frame; callers must not
    /// replay it because delivery may have been partial.
    pub(crate) fn send(
        &self,
        frame: &str,
        priority: Priority,
        timeout: Duration,
    ) -> Result<(), String> {
        let receiver = self.enqueue(frame, priority)?;
        receiver.recv_timeout(timeout).map_err(|_| {
            "Provider control frame was queued but not confirmed flushed in time.".to_string()
        })?
    }

    /// Enqueue without waiting for pipe I/O. The returned receiver can be
    /// dropped for best-effort cancellation, whose owner must still tear down
    /// the process independently to interrupt a blocked kernel write.
    pub(crate) fn enqueue(
        &self,
        frame: &str,
        priority: Priority,
    ) -> Result<mpsc::Receiver<Result<(), String>>, String> {
        let bytes = encode_line(frame)?;
        let (delivered, receiver) = mpsc::sync_channel(1);
        {
            let mut state = self
                .shared
                .state
                .lock()
                .map_err(|_| "Provider stdin writer state is unavailable.".to_string())?;
            if state.closed {
                return Err("Provider stdin writer is closed.".into());
            }
            let saturated = match priority {
                Priority::Normal => {
                    state.normal.len() >= NORMAL_QUEUE_CAPACITY
                        || state.queued_bytes.saturating_add(bytes.len()) > MAX_QUEUED_BYTES
                }
                Priority::Control => state.priority.len() >= PRIORITY_QUEUE_CAPACITY,
            };
            if saturated {
                return Err("Provider stdin writer queue is full.".into());
            }
            state.queued_bytes = state.queued_bytes.saturating_add(bytes.len());
            let item = Frame { bytes, delivered };
            match priority {
                Priority::Normal => state.normal.push_back(item),
                Priority::Control => state.priority.push_back(item),
            }
        }
        self.shared.ready.notify_one();
        Ok(receiver)
    }

    /// Closes admission immediately. It never waits for a blocked write.
    pub(crate) fn close(&self) {
        if let Ok(mut state) = self.shared.state.lock() {
            state.closed = true;
            for pending in state.normal.drain(..) {
                let _ = pending.delivered.send(Err(
                    "Provider stdin writer closed before the queued frame was written.".into(),
                ));
            }
            state.queued_bytes = state.priority.iter().map(|frame| frame.bytes.len()).sum();
        }
        self.shared.ready.notify_all();
    }
}

fn encode_line(frame: &str) -> Result<Vec<u8>, String> {
    if frame.len() > MAX_FRAME_BYTES {
        return Err(format!(
            "Provider control frame exceeds {MAX_FRAME_BYTES} bytes."
        ));
    }
    if frame.as_bytes().contains(&b'\n') || frame.as_bytes().contains(&b'\r') {
        return Err("Provider control frame contains a line break.".into());
    }
    let mut bytes = Vec::with_capacity(frame.len() + 1);
    bytes.extend_from_slice(frame.as_bytes());
    bytes.push(b'\n');
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io,
        sync::{Arc, Condvar, Mutex},
        time::Instant,
    };

    #[derive(Clone, Default)]
    struct GateWriter {
        state: Arc<(Mutex<GateState>, Condvar)>,
    }

    #[derive(Default)]
    struct GateState {
        blocked: bool,
        bytes: Vec<u8>,
    }

    impl Write for GateWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let (lock, changed) = &*self.state;
            let mut state = lock.lock().unwrap();
            while state.blocked {
                state = changed.wait(state).unwrap();
            }
            state.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn frame_is_reported_only_after_it_is_flushed_once() {
        let writer = GateWriter::default();
        let observed = Arc::clone(&writer.state);
        let transport = OutboundWriter::spawn(writer);
        transport
            .send("approval", Priority::Normal, Duration::from_secs(1))
            .unwrap();
        assert_eq!(observed.0.lock().unwrap().bytes, b"approval\n");
        transport.close();
    }

    #[test]
    fn pipe_error_is_returned_without_replaying_the_frame() {
        struct FailingWriter(Arc<Mutex<usize>>);
        impl Write for FailingWriter {
            fn write(&mut self, _bytes: &[u8]) -> io::Result<usize> {
                *self.0.lock().unwrap() += 1;
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let attempts = Arc::new(Mutex::new(0));
        let transport = OutboundWriter::spawn(FailingWriter(attempts.clone()));
        assert!(transport
            .send(
                "approval-response",
                Priority::Normal,
                Duration::from_secs(1)
            )
            .is_err());
        assert_eq!(*attempts.lock().unwrap(), 1);
        transport.close();
    }

    #[test]
    fn stop_close_is_bounded_while_pipe_write_is_blocked() {
        let writer = GateWriter::default();
        let observed = Arc::clone(&writer.state);
        observed.0.lock().unwrap().blocked = true;
        let transport = OutboundWriter::spawn(writer);
        let sender = transport.clone();
        let pending = thread::spawn(move || {
            sender.send("first", Priority::Normal, Duration::from_millis(80))
        });
        // Wait until the worker has consumed the first frame and is blocked in write.
        thread::sleep(Duration::from_millis(20));
        let started = Instant::now();
        transport.close();
        assert!(started.elapsed() < Duration::from_millis(100));
        assert!(pending.join().unwrap().is_err());
        let (lock, changed) = &*observed;
        lock.lock().unwrap().blocked = false;
        changed.notify_all();
    }

    #[test]
    fn priority_frames_precede_queued_normal_frames() {
        let writer = GateWriter::default();
        let observed = Arc::clone(&writer.state);
        observed.0.lock().unwrap().blocked = true;
        let transport = OutboundWriter::spawn(writer);
        let _inflight = transport.enqueue("inflight", Priority::Normal).unwrap();
        thread::sleep(Duration::from_millis(20));
        let _normal = transport.enqueue("normal", Priority::Normal).unwrap();
        let control = transport.enqueue("cancel", Priority::Control).unwrap();
        let started = Instant::now();
        let (lock, changed) = &*observed;
        lock.lock().unwrap().blocked = false;
        changed.notify_all();
        control
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap();
        assert!(started.elapsed() < Duration::from_secs(1));
        let _ = transport.send("barrier", Priority::Normal, Duration::from_secs(1));
        let bytes = &observed.0.lock().unwrap().bytes;
        let control_at = String::from_utf8_lossy(bytes).find("cancel\n").unwrap();
        let normal_at = String::from_utf8_lossy(bytes).find("normal\n").unwrap();
        assert!(control_at < normal_at);
        transport.close();
    }

    #[test]
    fn saturated_queue_rejects_without_growing_frame_memory() {
        let writer = GateWriter::default();
        let observed = Arc::clone(&writer.state);
        observed.0.lock().unwrap().blocked = true;
        let transport = OutboundWriter::spawn(writer);
        let _inflight = transport.enqueue("active", Priority::Normal).unwrap();
        thread::sleep(Duration::from_millis(20));
        for index in 0..NORMAL_QUEUE_CAPACITY {
            transport
                .enqueue(&format!("queued-{index}"), Priority::Normal)
                .unwrap();
        }
        assert!(transport.enqueue("overflow", Priority::Normal).is_err());
        let (lock, changed) = &*observed;
        lock.lock().unwrap().blocked = false;
        changed.notify_all();
        transport.close();
    }

    #[test]
    fn bounded_frames_reject_newlines_and_oversized_payloads() {
        let transport = OutboundWriter::spawn(io::sink());
        assert!(transport
            .send("one\ntwo", Priority::Normal, Duration::from_millis(10))
            .is_err());
        assert!(transport
            .send(
                &"x".repeat(MAX_FRAME_BYTES + 1),
                Priority::Normal,
                Duration::from_millis(10)
            )
            .is_err());
        transport.close();
    }
}
