use crate::{gettid, map_io_err};
use std::{
    cell::RefCell,
    fs::{File, OpenOptions},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
};

thread_local! {
    static MARKER_FILE: RefCell<Option<MarkerFile>> = const { RefCell::new(None) };
}

pub(super) struct FileLayer {
    pub(super) dir: Box<Path>,
}

impl FileLayer {
    pub(super) fn new(output_dir: Option<PathBuf>) -> io::Result<Self> {
        let dir = match &output_dir {
            Some(dir) => dir,
            None => &*std::env::temp_dir().join("tracing-samply"),
        };
        let dir = dir.join(std::process::id().to_string());
        if cfg!(unix) {
            std::fs::create_dir_all(&dir)
                .map_err(map_io_err("could not create perf markers dir", &dir))?;
        }
        Ok(Self { dir: dir.into_boxed_path() })
    }

    pub(super) fn on_exit(&self, start_ts: u64, end_ts: u64, name: &str) {
        MARKER_FILE.with_borrow_mut(|file| {
            let file = file.get_or_insert_with(|| self.create_marker_file());
            file.write_entry(start_ts, end_ts, name);
        });
    }

    fn create_marker_file(&self) -> MarkerFile {
        match self.try_create_marker_file() {
            Ok(file) => file,
            Err(err) => panic!("{err}"),
        }
    }

    fn try_create_marker_file(&self) -> io::Result<MarkerFile> {
        let pid = std::process::id();
        let fname = match gettid() {
            Some(tid) => format!("marker-{pid}-{tid}.txt"),
            None => format!("marker-{pid}.txt"),
        };
        let path = &*self.dir.join(fname);
        let file = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(path)
            .map_err(map_io_err("could not create perf markers file", path))?;
        // mmap the file to notify samply.
        // Linux perf needs `exec` permission to record it in perf.data.
        // On macOS, samply only needs the file to be opened, not mmap'ed.
        #[cfg(all(unix, not(target_vendor = "apple")))]
        let _ = unsafe {
            memmap2::MmapOptions::new()
                .map_exec(&file)
                .map_err(map_io_err("could not mmap perf markers file", path))?
        };
        Ok(MarkerFile::new(file))
    }
}

struct MarkerFile {
    file: BufWriter<File>,
}

impl MarkerFile {
    fn new(file: File) -> Self {
        Self { file: BufWriter::new(file) }
    }

    fn write_entry(&mut self, start_ts: u64, end_ts: u64, name: &str) {
        let _ = self.file.write_all(itoa::Buffer::new().format(start_ts).as_bytes());
        let _ = self.file.write_all(b" ");
        let _ = self.file.write_all(itoa::Buffer::new().format(end_ts).as_bytes());
        let _ = self.file.write_all(b" ");
        let _ = self.file.write_all(name.as_bytes());
        let _ = self.file.write_all(b"\n");
    }
}

pub fn flush_marker_file() {
    MARKER_FILE.with_borrow_mut(|file| {
        if let Some(file) = file {
            let _ = file.file.flush();
        }
    });
}
