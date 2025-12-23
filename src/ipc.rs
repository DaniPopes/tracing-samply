use crate::{gettid, map_io_err};
use std::{
    cell::RefCell,
    fs::{File, OpenOptions},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
};

pub(super) struct IpcLayer {
    pub(super) socket: Box<Path>,
}

impl IpcLayer {
    pub(super) fn new() -> io::Result<Self> {
        Ok(Self { socket: std::env::temp_dir().join("samply.sock").into() })
    }

    pub(super) fn on_exit(&self, start_ts: u64, end_ts: u64, name: &str) {
        todo!()
    }
}
