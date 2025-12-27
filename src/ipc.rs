use crate::{gettid, map_io_err};
use crossbeam_channel as mpsc;
use interprocess::local_socket::{GenericFilePath, Name, prelude::*};
use once_cell::sync::OnceCell as OnceLock;
use std::{
    io::{self, Write},
    sync::atomic::{AtomicUsize, Ordering},
    thread,
};

type Tx = mpsc::Sender<Message>;

pub(super) struct IpcLayer {
    name: Name<'static>,
    tx: OnceLock<Tx>,
    retry_counter: AtomicUsize,
}

enum Message {
    Span { tid: u64, start_ts: u64, end_ts: u64, name: &'static str },
}

const RETRY_PERIOD: usize = 1024;
const NO_RETRY: usize = usize::MAX / 2 + RETRY_PERIOD / 2;

impl IpcLayer {
    pub(super) fn new() -> io::Result<Self> {
        let path = std::env::temp_dir().join("samply.sock");
        let name = path.clone().to_fs_name::<GenericFilePath>()?;
        let this = Self { name, tx: OnceLock::new(), retry_counter: AtomicUsize::new(0) };
        let _ = this.maybe_connect().map_err(map_io_err("failed to connect to", &path))?;
        Ok(this)
    }

    pub(super) fn on_exit(&self, start_ts: u64, end_ts: u64, name: &'static str) {
        let Some(tx) = self.get() else { return };
        let Some(tid) = gettid() else { return };
        let _ = tx.send(Message::Span { tid, start_ts, end_ts, name });
    }

    fn get(&self) -> Option<&Tx> {
        if let Some(stream) = self.tx.get() {
            return Some(stream);
        }
        self.maybe_connect().ok().flatten()
    }

    #[cold]
    fn maybe_connect(&self) -> io::Result<Option<&Tx>> {
        let c = self.retry_counter.fetch_add(1, Ordering::Relaxed);
        if c == NO_RETRY {
            self.retry_counter.fetch_sub(1, Ordering::Relaxed);
        } else if c.is_multiple_of(RETRY_PERIOD) {
            let res = self.tx.get_or_try_init(|| self.connect());
            if let Err(err) = &res
                && !matches!(err.kind(), io::ErrorKind::NotFound)
            {
                self.retry_counter.store(NO_RETRY, Ordering::Relaxed);
                return res.map(Some);
            }
        }
        Ok(self.tx.get())
    }

    fn connect(&self) -> io::Result<Tx> {
        let mut stream = io::BufWriter::new(LocalSocketStream::connect(self.name.borrow())?);
        stream.get_ref().set_nonblocking(true)?;
        writeln!(stream, r#"{{"type":"Init","pid":{}}}"#, std::process::id())?;
        stream.flush()?;

        let (tx, rx) = mpsc::unbounded::<Message>();
        thread::Builder::new().name("tracing-samply".into()).spawn(move || {
            for msg in rx {
                match msg {
                    Message::Span { tid, start_ts, end_ts, name } => {
                        let _ = writeln!(stream, r#"{{"type":"Span","tid":{tid},"start":{start_ts},"end":{end_ts},"label":"{name}"}}"#);
                    }
                }
            }
            let _ = stream.flush();
        })?;

        Ok(tx)
    }
}
