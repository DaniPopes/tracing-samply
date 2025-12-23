#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]

use smallvec::{SmallVec, smallvec};
use std::{
    io::{self},
    path::{Path, PathBuf},
};
use tracing_core::{Subscriber, span};
use tracing_subscriber::{Layer, layer::Context, registry::LookupSpan};

mod file;
use file::FileLayer;

mod ipc;
use ipc::IpcLayer;

const IPC: bool = true;

/// [`SamplyLayer`] builder.
///
/// See the [crate docs](crate) for more information.
pub struct SamplyLayerBuilder {
    output_dir: Option<PathBuf>,
}

impl Default for SamplyLayerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl SamplyLayerBuilder {
    /// Creates a new [`SamplyLayerBuilder`].
    pub fn new() -> Self {
        Self { output_dir: None }
    }

    /// Sets the output directory for intermediate files.
    ///
    /// If unset, a temporary directory will be created and used.
    pub fn output_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.output_dir = Some(dir.into());
        self
    }

    /// Builds a new [`SamplyLayer`].
    pub fn build(self) -> io::Result<SamplyLayer> {
        let Self { output_dir } = self;
        Ok(SamplyLayer { imp: Impl::File(FileLayer::new(output_dir)?) })
    }
}

/// A tracing layer that bridges `tracing` events and spans with `samply`.
///
/// See the [crate docs](crate) for more information.
pub struct SamplyLayer {
    imp: Impl,
}

struct SpanDataStack {
    stack: SmallVec<[SpanData; 1]>,
}
struct SpanData {
    start_ts: u64,
}

impl SamplyLayer {
    /// Creates a new [`SamplyLayer`].
    ///
    /// This is the same as `SamplyLayer::builder().build()`.
    pub fn new() -> io::Result<Self> {
        Self::builder().build()
    }

    /// Creates a new [`SamplyLayer`] builder.
    pub fn builder() -> SamplyLayerBuilder {
        SamplyLayerBuilder::new()
    }
}

/// A tracing layer that bridges `tracing` events and spans with `samply`.
///
/// See the [crate docs](crate) for more information.
enum Impl {
    File(FileLayer),
    Ipc(IpcLayer),
}

impl<S> Layer<S> for SamplyLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_enter(&self, id: &span::Id, ctx: Context<'_, S>) {
        if !cfg!(unix) {
            return;
        }
        let Some(span) = ctx.span(id) else { return };
        let data = SpanData { start_ts: now_timestamp() };
        let mut extensions = span.extensions_mut();
        if let Some(stack) = extensions.get_mut::<SpanDataStack>() {
            stack.stack.push(data);
        } else {
            extensions.insert(SpanDataStack { stack: smallvec![data] });
        }
    }

    fn on_exit(&self, id: &span::Id, ctx: Context<'_, S>) {
        if !cfg!(unix) {
            return;
        }
        let Some(span) = ctx.span(id) else { return };
        let mut extensions = span.extensions_mut();
        let Some(data) = extensions.get_mut::<SpanDataStack>() else { return };
        let Some(SpanData { start_ts }) = data.stack.pop() else { return };
        let end_ts = now_timestamp();
        match &self.imp {
            Impl::File(f) => f.on_exit(start_ts, end_ts, span.name()),
            Impl::Ipc(i) => i.on_exit(start_ts, end_ts, span.name()),
        }
    }
}

fn now_timestamp() -> u64 {
    cfg_if::cfg_if! {
        if #[cfg(target_vendor = "apple")] {
            // https://github.com/mstange/samply/blob/2041b956f650bb92d912990052967d03fef66b75/samply/src/mac/time.rs#L7
            use std::sync::OnceLock;
            use mach2::mach_time;

            static NANOS_PER_TICK: OnceLock<mach_time::mach_timebase_info> = OnceLock::new();

            let nanos_per_tick = NANOS_PER_TICK.get_or_init(|| unsafe {
                let mut info = mach_time::mach_timebase_info::default();
                let errno = mach_time::mach_timebase_info(&mut info as *mut _);
                if errno != 0 || info.denom == 0 {
                    info.numer = 1;
                    info.denom = 1;
                };
                info
            });

            let time = unsafe { mach_time::mach_absolute_time() };

            time * nanos_per_tick.numer as u64 / nanos_per_tick.denom as u64
        } else if #[cfg(unix)] {
            let mut ts = unsafe { std::mem::zeroed() };
            if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) } != 0 {
                return u64::MAX;
            }
            std::time::Duration::new(ts.tv_sec as _, ts.tv_nsec as _)
                .as_nanos()
                .try_into()
                .unwrap_or(u64::MAX)
        } else {
            0
        }
    }
}

fn gettid() -> Option<u64> {
    // https://github.com/rust-lang/rust/blob/9044e98b66d074e7f88b1d4cea58bb0538f2eda6/library/std/src/sys/thread/unix.rs#L325
    cfg_if::cfg_if! {
        if #[cfg(target_vendor = "apple")] {
            let mut tid = 0u64;
            let status = unsafe { libc::pthread_threadid_np(0, &mut tid) };
            (status == 0).then_some(tid)
        } else if #[cfg(unix)] {
            Some(unsafe { libc::gettid() } as u64)
        // } else if #[cfg(windows)] {
        //     let tid = unsafe { c::GetCurrentThreadId() } as u64;
        //     if tid == 0 { None } else { Some(tid as _) }
        } else {
            None
        }
    }
}

fn map_io_err(s: &str, p: &Path) -> impl FnOnce(io::Error) -> io::Error {
    move |e| io::Error::new(e.kind(), format!("{s} {p:?}: {e}"))
}

// Not public API. Only for testing purposes.
#[doc(hidden)]
pub mod __private {
    use super::*;

    pub use file::flush_marker_file;
}
