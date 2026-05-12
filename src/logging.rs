// sincronia/src/logging.rs
//
// Sistema de logging triple: log humano (.log), métricas CSV, eventos JSONL.
// Usa tracing para el log humano y escritores especializados para CSV/JSONL.

use crate::errors::FileState;
use chrono::Local;
use parking_lot::Mutex;
use serde::Serialize;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::fmt;
use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;

// ─────────────────────────────────────────────
// Métricas por archivo
// ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct FileMetrics {
    pub timestamp: String,
    pub cycle_id: String,
    pub relative_path: String,
    pub source_path: String,
    pub destination_path: String,
    pub file_size_bytes: u64,
    pub copy_duration_ms: u64,
    pub hash_duration_ms: u64,
    pub total_duration_ms: u64,
    pub average_speed_mbps: f64,
    pub retry_count: u32,
    pub final_state: String,
    pub error_message_if_any: String,
}

// ─────────────────────────────────────────────
// Métricas por ciclo
// ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct CycleMetrics {
    pub timestamp: String,
    pub cycle_id: String,
    pub files_detected: u64,
    pub files_stable: u64,
    pub files_unstable: u64,
    pub files_copied: u64,
    pub files_already_backed_up: u64,
    pub files_versioned: u64,
    pub files_conflicted: u64,
    pub files_failed: u64,
    pub bytes_copied: u64,
    pub cycle_duration_seconds: f64,
    pub average_speed_mbps: f64,
    pub peak_speed_mbps: f64,
    pub consecutive_errors: u32,
}

// ─────────────────────────────────────────────
// Métricas acumuladas
// ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct AccumulatedMetrics {
    pub uptime_seconds: f64,
    pub total_files_processed: u64,
    pub total_files_copied: u64,
    pub total_files_failed: u64,
    pub total_bytes_copied: u64,
    pub global_average_speed_mbps: f64,
    pub current_nas_status: String,
    pub current_queue_depth: u64,
}

// ─────────────────────────────────────────────
// Evento estructurado para JSONL
// ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct StructuredEvent {
    pub timestamp: String,
    pub level: String,
    pub event_type: String,
    pub cycle_id: Option<String>,
    pub file_path: Option<String>,
    pub file_state: Option<FileState>,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed_mbps: Option<f64>,
}

impl StructuredEvent {
    pub fn new(level: &str, event_type: &str, message: &str) -> Self {
        Self {
            timestamp: Local::now().format("%Y-%m-%dT%H:%M:%S%.3f%:z").to_string(),
            level: level.to_string(),
            event_type: event_type.to_string(),
            cycle_id: None,
            file_path: None,
            file_state: None,
            message: message.to_string(),
            error: None,
            size_bytes: None,
            duration_ms: None,
            speed_mbps: None,
        }
    }

    pub fn with_cycle(mut self, cycle_id: &str) -> Self {
        self.cycle_id = Some(cycle_id.to_string());
        self
    }

    pub fn with_file(mut self, path: &str, state: FileState) -> Self {
        self.file_path = Some(path.to_string());
        self.file_state = Some(state);
        self
    }

    pub fn with_error(mut self, error: &str) -> Self {
        self.error = Some(error.to_string());
        self
    }

    pub fn with_metrics(mut self, size: u64, duration_ms: u64, speed: f64) -> Self {
        self.size_bytes = Some(size);
        self.duration_ms = Some(duration_ms);
        self.speed_mbps = Some(speed);
        self
    }
}

// ─────────────────────────────────────────────
// Gestor de logging
// ─────────────────────────────────────────────

pub struct LogManager {
    csv_writer: Mutex<Option<csv::Writer<BufWriter<File>>>>,
    jsonl_writer: Mutex<Option<BufWriter<File>>>,
    csv_path: PathBuf,
    jsonl_path: PathBuf,
    // Guard must be kept alive for tracing to flush on drop
    _tracing_guard: Option<WorkerGuard>,
}

impl LogManager {
    /// Inicializa el sistema de logging triple
    pub fn init(
        human_log_path: &Path,
        csv_path: &Path,
        jsonl_path: &Path,
    ) -> anyhow::Result<Self> {
        // ── Tracing para log humano ──
        let log_dir = human_log_path
            .parent()
            .unwrap_or(Path::new("."));
        let log_filename = human_log_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let file_appender = tracing_appender::rolling::daily(log_dir, &log_filename);
        let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

        let subscriber = tracing_subscriber::registry()
            .with(
                EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| EnvFilter::new("info")),
            )
            .with(
                fmt::layer()
                    .with_writer(non_blocking)
                    .with_ansi(false)
                    .with_target(true)
                    .with_thread_ids(true),
            )
            .with(
                fmt::layer()
                    .with_writer(std::io::stderr)
                    .with_ansi(true)
                    .with_target(false),
            );

        tracing::subscriber::set_global_default(subscriber)
            .map_err(|e| anyhow::anyhow!("Error al configurar tracing: {}", e))?;

        // ── CSV writer ──
        let csv_writer = Self::open_csv_writer(csv_path)?;

        // ── JSONL writer ──
        let jsonl_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(jsonl_path)
            .map_err(|e| anyhow::anyhow!("No se pudo abrir JSONL '{}': {}", jsonl_path.display(), e))?;
        let jsonl_writer = BufWriter::new(jsonl_file);

        Ok(Self {
            csv_writer: Mutex::new(Some(csv_writer)),
            jsonl_writer: Mutex::new(Some(jsonl_writer)),
            csv_path: csv_path.to_path_buf(),
            jsonl_path: jsonl_path.to_path_buf(),
            _tracing_guard: Some(guard),
        })
    }

    fn open_csv_writer(path: &Path) -> anyhow::Result<csv::Writer<BufWriter<File>>> {
        let file_existed = path.exists() && path.metadata().map(|m| m.len() > 0).unwrap_or(false);

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| anyhow::anyhow!("No se pudo abrir CSV '{}': {}", path.display(), e))?;

        let buf_writer = BufWriter::new(file);
        let mut csv_writer = csv::WriterBuilder::new()
            .has_headers(!file_existed)
            .from_writer(buf_writer);

        // Si el archivo es nuevo, escribir cabecera
        if !file_existed {
            csv_writer.write_record([
                "timestamp",
                "cycle_id",
                "relative_path",
                "source_path",
                "destination_path",
                "file_size_bytes",
                "copy_duration_ms",
                "hash_duration_ms",
                "total_duration_ms",
                "average_speed_MBps",
                "retry_count",
                "final_state",
                "error_message_if_any",
            ])?;
            csv_writer.flush()?;
        }

        Ok(csv_writer)
    }

    /// Registra métricas de un archivo en el CSV
    pub fn write_file_metrics(&self, metrics: &FileMetrics) -> anyhow::Result<()> {
        let mut guard = self.csv_writer.lock();
        if let Some(writer) = guard.as_mut() {
            writer.serialize(metrics)?;
            writer.flush()?;
        }
        Ok(())
    }

    /// Registra un evento estructurado en el JSONL
    pub fn write_event(&self, event: &StructuredEvent) -> anyhow::Result<()> {
        let mut guard = self.jsonl_writer.lock();
        if let Some(writer) = guard.as_mut() {
            let json = serde_json::to_string(event)?;
            writeln!(writer, "{}", json)?;
            writer.flush()?;
        }
        Ok(())
    }

    /// Registra métricas de ciclo como evento JSONL
    pub fn write_cycle_metrics(&self, metrics: &CycleMetrics) -> anyhow::Result<()> {
        let event = StructuredEvent {
            timestamp: metrics.timestamp.clone(),
            level: "INFO".to_string(),
            event_type: "cycle_completed".to_string(),
            cycle_id: Some(metrics.cycle_id.clone()),
            file_path: None,
            file_state: None,
            message: format!(
                "Ciclo completado: {} archivos copiados, {} bytes, {:.1} MB/s",
                metrics.files_copied, metrics.bytes_copied, metrics.average_speed_mbps
            ),
            error: None,
            size_bytes: Some(metrics.bytes_copied),
            duration_ms: Some((metrics.cycle_duration_seconds * 1000.0) as u64),
            speed_mbps: Some(metrics.average_speed_mbps),
        };
        self.write_event(&event)
    }

    /// Devuelve la ruta del archivo CSV
    pub fn csv_path(&self) -> &Path {
        &self.csv_path
    }

    /// Devuelve la ruta del archivo JSONL
    pub fn jsonl_path(&self) -> &Path {
        &self.jsonl_path
    }
}
