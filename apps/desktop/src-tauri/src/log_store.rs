use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tracing_subscriber::fmt::{self, MakeWriter};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::Registry;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

const MAX_LINES: usize = 2_000;

#[derive(Clone)]
pub struct LogStore {
    lines: Arc<Mutex<VecDeque<String>>>,
}

impl LogStore {
    pub fn new() -> Self {
        Self {
            lines: Arc::new(Mutex::new(VecDeque::with_capacity(MAX_LINES))),
        }
    }

    pub fn push_line(&self, line: impl Into<String>) {
        let line = line.into();
        if line.is_empty() {
            return;
        }
        if let Ok(mut guard) = self.lines.lock() {
            if guard.len() >= MAX_LINES {
                guard.pop_front();
            }
            guard.push_back(line);
        }
    }

    pub fn recent(&self, limit: usize) -> Vec<String> {
        let guard = self.lines.lock().ok();
        guard
            .map(|g| {
                let n = limit.min(g.len());
                g.iter().skip(g.len().saturating_sub(n)).cloned().collect()
            })
            .unwrap_or_default()
    }

    pub fn clear(&self) {
        if let Ok(mut guard) = self.lines.lock() {
            guard.clear();
        }
    }
}

#[derive(Clone)]
struct MemoryMakeWriter {
    store: LogStore,
}

impl<'a> MakeWriter<'a> for MemoryMakeWriter {
    type Writer = MemoryWriter;

    fn make_writer(&'a self) -> Self::Writer {
        MemoryWriter {
            store: self.store.clone(),
            buf: String::new(),
        }
    }
}

struct MemoryWriter {
    store: LogStore,
    buf: String,
}

impl Write for MemoryWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.buf.push_str(&String::from_utf8_lossy(buf));
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let text = self.buf.trim_end();
        if !text.is_empty() {
            for line in text.lines() {
                self.store.push_line(line);
            }
        }
        self.buf.clear();
        Ok(())
    }
}

pub struct LogBootstrap {
    pub _file_guard: tracing_appender::non_blocking::WorkerGuard,
}

pub fn init_tracing(log_dir: &Path, store: LogStore) -> Result<(LogBootstrap, PathBuf), String> {
    fs::create_dir_all(log_dir).map_err(|e| e.to_string())?;

    let log_file = log_dir.join("lan-screenshare.log");
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file)
        .map_err(|e| format!("open log file: {e}"))?;

    let (file_writer, file_guard) = tracing_appender::non_blocking(file);

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new(
            "info,lan_share_stream=debug,lan_share_capture=info,lan_screenshare_desktop_lib=debug",
        )
    });

    let console_layer = fmt::layer()
        .with_ansi(cfg!(debug_assertions))
        .with_writer(std::io::stderr);

    let file_layer = fmt::layer().with_ansi(false).with_writer(file_writer);

    let mem_layer = fmt::layer().with_ansi(false).with_writer(MemoryMakeWriter {
        store: store.clone(),
    });

    Registry::default()
        .with(filter)
        .with(console_layer)
        .with(file_layer)
        .with(mem_layer)
        .try_init()
        .map_err(|e| format!("init tracing: {e}"))?;

    Ok((LogBootstrap { _file_guard: file_guard }, log_file))
}
