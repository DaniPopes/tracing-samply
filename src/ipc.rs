use crate::gettid;
use interprocess::local_socket::{GenericFilePath, Name, prelude::*};
use once_cell::sync::OnceCell as OnceLock;
use std::{
    io::{self, BufWriter, Write},
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

type Stream = Mutex<BufWriter<LocalSocketStream>>;

pub(super) struct IpcLayer {
    name: Name<'static>,
    stream: OnceLock<Stream>,
    retry_counter: AtomicUsize,
}

const RETRY_PERIOD: usize = 1024;
const NO_RETRY: usize = usize::MAX / 2 + RETRY_PERIOD / 2;

impl IpcLayer {
    pub(super) fn new() -> io::Result<Self> {
        let path = std::env::temp_dir().join("samply.sock");
        let name = path.to_fs_name::<GenericFilePath>()?;
        let this = Self { name, stream: OnceLock::new(), retry_counter: AtomicUsize::new(0) };
        let _ = this.maybe_connect()?;
        Ok(this)
    }

    pub(super) fn on_exit(&self, start_ts: u64, end_ts: u64, name: &'static str) {
        let Some(stream) = self.get() else { return };
        let Some(tid) = gettid() else { return };
        let mut stream = stream.lock().unwrap();
        let _ = writeln!(
            stream,
            r#"{{"type":"Span","tid":{tid},"start":{start_ts},"end":{end_ts},"label":"{name}"}}"#
        );
    }

    fn get(&self) -> Option<&Stream> {
        if let Some(stream) = self.stream.get() {
            return Some(stream);
        }
        self.maybe_connect().ok().flatten()
    }

    #[cold]
    fn maybe_connect(&self) -> io::Result<Option<&Stream>> {
        let c = self.retry_counter.fetch_add(1, Ordering::Relaxed);
        if c == NO_RETRY {
            self.retry_counter.fetch_sub(1, Ordering::Relaxed);
        } else if c.is_multiple_of(RETRY_PERIOD) {
            let res = self.stream.get_or_try_init(|| self.connect());
            if let Err(err) = &res
                && !matches!(err.kind(), io::ErrorKind::NotFound)
            {
                self.retry_counter.store(NO_RETRY, Ordering::Relaxed);
                return res.map(Some);
            }
        }
        Ok(self.stream.get())
    }

    fn connect(&self) -> io::Result<Stream> {
        let mut stream = BufWriter::new(LocalSocketStream::connect(self.name.borrow())?);
        writeln!(stream, r#"{{"type":"Init","pid":{}}}"#, std::process::id())?;
        Ok(Mutex::new(stream))
    }
}
