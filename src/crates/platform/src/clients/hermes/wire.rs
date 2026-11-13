//! The TCP stream under one WebSocket, shared with whoever may have to
//! cut it. Cutting is how an answer on its way is stopped from another
//! thread: the stream is shut down, so a write that waits on the gateway
//! ends at once, and from then on nothing more is written through it,
//! neither by a later call nor by the WebSocket flushing what an earlier
//! write left unsent. What the kernel already took of a frame that was
//! cut still reaches the gateway, followed by the end of the stream: a
//! piece of a frame, which is no frame. The stream is not reset: measured
//! on macOS, a gateway that had stopped reading now and then never learnt
//! of a reset at all and kept the dead socket, while it always sees the
//! stream end.
use std::{
    io::{self, ErrorKind, Read, Write},
    net::{Shutdown, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, PoisonError,
    },
    time::{Duration, Instant},
};

pub(super) struct Line {
    tcp: TcpStream,
    cut: AtomicBool,
    /// When the frame being written has to be out: a gateway that reads a
    /// little now and then cannot keep one write going past it.
    out_by: Mutex<Option<Instant>>,
}

impl Line {
    pub(super) fn new(tcp: TcpStream) -> Arc<Self> {
        Arc::new(Self {
            tcp,
            cut: AtomicBool::new(false),
            out_by: Mutex::new(None),
        })
    }

    /// Cuts the stream, from any thread. Nothing is written after this.
    pub(super) fn cut(&self) {
        if self.cut.swap(true, Ordering::AcqRel) {
            return;
        }
        let _ = self.tcp.shutdown(Shutdown::Both);
    }

    pub(super) fn is_cut(&self) -> bool {
        self.cut.load(Ordering::Acquire)
    }

    /// Bounds the writes that follow: all of them together end by `time`
    /// from now. None lifts the bound.
    pub(super) fn out_within(&self, time: Option<Duration>) {
        let by = time.map(|time| Instant::now() + time);
        *self.out_by.lock().unwrap_or_else(PoisonError::into_inner) = by;
    }

    pub(super) fn wait_to_read(&self, time: Duration) {
        let _ = self.tcp.set_read_timeout(Some(time));
    }

    /// How long the next write may wait, or the error that ends it.
    fn may_write(&self) -> io::Result<Option<Duration>> {
        if self.is_cut() {
            return Err(ErrorKind::ConnectionAborted.into());
        }
        let by = *self.out_by.lock().unwrap_or_else(PoisonError::into_inner);
        match by.map(|by| by.saturating_duration_since(Instant::now())) {
            Some(left) if left.is_zero() => Err(ErrorKind::TimedOut.into()),
            left => Ok(left),
        }
    }
}

/// The stream as the WebSocket reads and writes it.
pub(super) struct Wire(pub(super) Arc<Line>);

impl Read for Wire {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        (&self.0.tcp).read(bytes)
    }
}

impl Write for Wire {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if let Some(left) = self.0.may_write()? {
            let _ = self.0.tcp.set_write_timeout(Some(left));
        }
        (&self.0.tcp).write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.may_write()?;
        (&self.0.tcp).flush()
    }
}
