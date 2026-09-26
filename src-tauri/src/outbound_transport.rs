//! Bounded, single-writer transport for line-framed child stdin pipes.
//!
//! Provider adapters still own the protocol JSON. This module only owns the
//! byte pipe, queue bounds, flush acknowledgements, and cancellation priority.

use std::{
    collections::VecDeque,
    io::Write,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc, Condvar, Mutex,
    },
    thread,
    time::Duration,
};

/// Leaves room for a JSON frame containing a maximum-size stored attachment
/// encoded as base64 (20 MiB raw, about 26.7 MiB encoded) plus protocol text.
/// ACP and Codex keep their separate 2 MiB inbound-reader limits.
pub(crate) const MAX_FRAME_BYTES: usize = 32 * 1024 * 1024;
const NORMAL_QUEUE_CAPACITY: usize = 64;
const MAX_QUEUED_BYTES: usize = 64 * 1024 * 1024;
const PRIORITY_QUEUE_CAPACITY: usize = 4;

pub(crate) struct OutboundWriter {
    shared: Arc<Shared>,
}

struct Shared {
    state: Mutex<State>,
    ready: Condvar,
    owners: AtomicUsize,
    writing: AtomicUsize,
}

struct State {
    normal: VecDeque<Frame>,
    priority: VecDeque<Frame>,
    queued_bytes: usize,
    closed: bool,
    next_ticket: u64,
}

struct Frame {
    ticket: u64,
    bytes: Vec<u8>,
    delivered: mpsc::SyncSender<Result<(), String>>,
}

pub(crate) struct Delivery {
    ticket: u64,
    receiver: mpsc::Receiver<Result<(), String>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Priority {
    Normal,
    Control,
}

impl OutboundWriter {
    /// Starts the sole writer for a child stdin pipe. The worker does not count
    /// as a sender owner, so dropping the final handle closes the queue.
    pub(crate) fn spawn<W>(mut writer: W) -> Result<Self, String>
    where
        W: Write + Send + 'static,
    {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                normal: VecDeque::new(),
                priority: VecDeque::new(),
                queued_bytes: 0,
                closed: false,
                next_ticket: 1,
            }),
            ready: Condvar::new(),
            owners: AtomicUsize::new(1),
            writing: AtomicUsize::new(0),
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
                worker_shared.writing.fetch_add(1, Ordering::Release);
                let result = writer
                    .write_all(&frame.bytes)
                    .and_then(|_| writer.flush())
                    .map_err(|error| format!("Could not flush provider control frame: {error}"));
                worker_shared.writing.fetch_sub(1, Ordering::Release);
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
            .map_err(|error| format!("Could not start provider stdin writer: {error}"))?;
        Ok(Self { shared })
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
        let delivery = self.enqueue(frame, priority)?;
        match delivery.receiver.recv_timeout(timeout) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if self.cancel_if_queued(delivery.ticket) {
                    Err("Provider control frame timed out and was removed before writing.".into())
                } else {
                    Err("Provider control frame timed out with delivery still unknown.".into())
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                Err("Provider stdin writer stopped before confirming delivery.".into())
            }
        }
    }

    /// Enqueue without waiting for pipe I/O. The returned receiver can be
    /// dropped for best-effort cancellation, whose owner must still tear down
    /// the process independently to interrupt a blocked kernel write.
    pub(crate) fn enqueue(&self, frame: &str, priority: Priority) -> Result<Delivery, String> {
        let bytes = encode_line(frame)?;
        let (delivered, receiver) = mpsc::sync_channel(1);
        let ticket;
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
            if saturated || state.queued_bytes.saturating_add(bytes.len()) > MAX_QUEUED_BYTES {
                return Err("Provider stdin writer queue is full.".into());
            }
            ticket = state.next_ticket;
            state.next_ticket = state.next_ticket.wrapping_add(1).max(1);
            state.queued_bytes = state.queued_bytes.saturating_add(bytes.len());
            let item = Frame {
                ticket,
                bytes,
                delivered,
            };
            match priority {
                Priority::Normal => state.normal.push_back(item),
                Priority::Control => state.priority.push_back(item),
            }
        }
        self.shared.ready.notify_one();
        Ok(Delivery { ticket, receiver })
    }

    fn cancel_if_queued(&self, ticket: u64) -> bool {
        let Ok(mut state) = self.shared.state.lock() else {
            return false;
        };
        let frame = remove_ticket(&mut state.priority, ticket)
            .or_else(|| remove_ticket(&mut state.normal, ticket));
        if let Some(frame) = frame {
            state.queued_bytes = state.queued_bytes.saturating_sub(frame.bytes.len());
            let _ = frame.delivered.send(Err(
                "Provider control frame timed out before writing.".into()
            ));
            return true;
        }
        false
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

fn remove_ticket(queue: &mut VecDeque<Frame>, ticket: u64) -> Option<Frame> {
    let index = queue.iter().position(|frame| frame.ticket == ticket)?;
    queue.remove(index)
}

impl Clone for OutboundWriter {
    fn clone(&self) -> Self {
        self.shared.owners.fetch_add(1, Ordering::Relaxed);
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl Drop for OutboundWriter {
    fn drop(&mut self) {
        if self.shared.owners.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.close();
        }
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
        let transport = OutboundWriter::spawn(writer).unwrap();
        transport
            .send("approval", Priority::Normal, Duration::from_secs(1))
            .unwrap();
        assert_eq!(observed.0.lock().unwrap().bytes, b"approval\n");
        transport.close();
    }

    #[test]
    fn dropping_only_one_clone_keeps_writer_open_but_final_drop_exits() {
        struct DropSignalWriter(Option<mpsc::Sender<()>>);
        impl Write for DropSignalWriter {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        impl Drop for DropSignalWriter {
            fn drop(&mut self) {
                if let Some(sender) = self.0.take() {
                    let _ = sender.send(());
                }
            }
        }

        let (dropped, observed) = mpsc::channel();
        let transport = OutboundWriter::spawn(DropSignalWriter(Some(dropped))).unwrap();
        let final_handle = transport.clone();
        drop(transport);
        assert!(observed.try_recv().is_err());
        final_handle
            .send("still-open", Priority::Normal, Duration::from_secs(1))
            .unwrap();
        drop(final_handle);
        observed.recv_timeout(Duration::from_secs(1)).unwrap();
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
        let transport = OutboundWriter::spawn(FailingWriter(attempts.clone())).unwrap();
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
    fn timeout_removes_a_frame_that_has_not_started_writing() {
        let writer = GateWriter::default();
        let observed = Arc::clone(&writer.state);
        observed.0.lock().unwrap().blocked = true;
        let transport = OutboundWriter::spawn(writer).unwrap();
        let first = transport.enqueue("inflight", Priority::Normal).unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        while transport.shared.writing.load(Ordering::Acquire) == 0 && Instant::now() < deadline {
            thread::yield_now();
        }
        let error = transport
            .send("approval", Priority::Normal, Duration::from_millis(30))
            .unwrap_err();
        assert!(error.contains("removed before writing"));
        let (lock, changed) = &*observed;
        lock.lock().unwrap().blocked = false;
        changed.notify_all();
        first
            .receiver
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap();
        thread::sleep(Duration::from_millis(20));
        assert_eq!(observed.0.lock().unwrap().bytes, b"inflight\n");
        transport.close();
    }

    #[test]
    fn timeout_of_inflight_frame_is_ambiguous_and_never_retried() {
        let writer = GateWriter::default();
        let observed = Arc::clone(&writer.state);
        observed.0.lock().unwrap().blocked = true;
        let transport = OutboundWriter::spawn(writer).unwrap();
        let sender = transport.clone();
        let pending = thread::spawn(move || {
            sender.send("approval", Priority::Normal, Duration::from_millis(40))
        });
        let deadline = Instant::now() + Duration::from_secs(1);
        while transport.shared.writing.load(Ordering::Acquire) == 0 && Instant::now() < deadline {
            thread::yield_now();
        }
        assert_eq!(transport.shared.writing.load(Ordering::Acquire), 1);
        assert!(pending.join().unwrap().unwrap_err().contains("unknown"));
        let (lock, changed) = &*observed;
        lock.lock().unwrap().blocked = false;
        changed.notify_all();
        let barrier = transport.enqueue("barrier", Priority::Normal).unwrap();
        barrier
            .receiver
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap();
        assert_eq!(observed.0.lock().unwrap().bytes, b"approval\nbarrier\n");
        transport.close();
    }

    #[test]
    fn stop_close_is_bounded_while_pipe_write_is_blocked() {
        let writer = GateWriter::default();
        let observed = Arc::clone(&writer.state);
        observed.0.lock().unwrap().blocked = true;
        let transport = OutboundWriter::spawn(writer).unwrap();
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
        let transport = OutboundWriter::spawn(writer).unwrap();
        let _inflight = transport.enqueue("inflight", Priority::Normal).unwrap();
        thread::sleep(Duration::from_millis(20));
        let _normal = transport.enqueue("normal", Priority::Normal).unwrap();
        let control = transport.enqueue("cancel", Priority::Control).unwrap();
        let started = Instant::now();
        let (lock, changed) = &*observed;
        lock.lock().unwrap().blocked = false;
        changed.notify_all();
        control
            .receiver
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
        let transport = OutboundWriter::spawn(writer).unwrap();
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
        let writer = GateWriter::default();
        let observed = Arc::clone(&writer.state);
        let transport = OutboundWriter::spawn(writer).unwrap();
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
        assert!(observed.0.lock().unwrap().bytes.is_empty());
        transport.close();
    }

    #[test]
    fn maximum_frame_fits_the_shared_queue_byte_budget() {
        let writer = GateWriter::default();
        let observed = Arc::clone(&writer.state);
        observed.0.lock().unwrap().blocked = true;
        let transport = OutboundWriter::spawn(writer).unwrap();
        let delivery = transport
            .enqueue(&"x".repeat(MAX_FRAME_BYTES), Priority::Normal)
            .unwrap();
        {
            let state = transport.shared.state.lock().unwrap();
            assert!(state.queued_bytes <= MAX_QUEUED_BYTES);
        }
        let (lock, changed) = &*observed;
        lock.lock().unwrap().blocked = false;
        changed.notify_all();
        assert_eq!(
            delivery
                .receiver
                .recv_timeout(Duration::from_secs(2))
                .unwrap(),
            Ok(())
        );
        transport.close();
    }

    #[cfg(unix)]
    #[test]
    fn blocked_child_stdin_is_released_by_process_teardown() {
        use std::{
            io::BufRead,
            process::{Command, Stdio},
        };

        let mut child = Command::new("sh")
            .args(["-c", "printf 'ready\\n'; exec sleep 30"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut ready = String::new();
        std::io::BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut ready)
            .unwrap();
        assert_eq!(ready.trim(), "ready");

        let transport = OutboundWriter::spawn(child.stdin.take().unwrap()).unwrap();
        let writer = transport.clone();
        let frame = "x".repeat(MAX_FRAME_BYTES);
        let pending =
            thread::spawn(move || writer.send(&frame, Priority::Normal, Duration::from_secs(5)));
        let deadline = Instant::now() + Duration::from_secs(1);
        while transport.shared.writing.load(Ordering::Acquire) == 0 && Instant::now() < deadline {
            thread::yield_now();
        }
        assert_eq!(transport.shared.writing.load(Ordering::Acquire), 1);
        thread::sleep(Duration::from_millis(50));
        transport.close();
        assert!(
            !pending.is_finished(),
            "close must not wait for or fake completion of the pipe write"
        );
        let started = Instant::now();
        child.kill().unwrap();
        let _ = child.wait().unwrap();
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(pending.join().unwrap().is_err());
        drop(transport);
    }
}
