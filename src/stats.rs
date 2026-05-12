// sincronia/src/stats.rs
//
// Agregador de métricas por archivo, ciclo y acumuladas.
// Alimenta los writers de CSV y JSONL del LogManager.

use crate::errors::FileState;
use crate::logging::{AccumulatedMetrics, CycleMetrics, FileMetrics, LogManager};
use crate::scheduler::JobResult;
use chrono::Local;
use parking_lot::RwLock;
use std::sync::Arc;
use std::time::Instant;
use tracing::debug;

/// Agregador de estadísticas del sistema
pub struct StatsAggregator {
    /// Métricas acumuladas (compartidas con tray vía Arc)
    accumulated: Arc<RwLock<AccumulatedMetrics>>,
    /// Instante de inicio del sistema
    start_time: Instant,
    /// Contador de ciclos
    cycle_counter: u64,
    /// Errores consecutivos actual
    pub consecutive_errors: u32,
}

impl StatsAggregator {
    pub fn new() -> Self {
        Self {
            accumulated: Arc::new(RwLock::new(AccumulatedMetrics {
                uptime_seconds: 0.0,
                total_files_processed: 0,
                total_files_copied: 0,
                total_files_failed: 0,
                total_bytes_copied: 0,
                global_average_speed_mbps: 0.0,
                current_nas_status: "unknown".into(),
                current_queue_depth: 0,
            })),
            start_time: Instant::now(),
            cycle_counter: 0,
            consecutive_errors: 0,
        }
    }

    /// Referencia compartida a métricas acumuladas (para tray UI)
    pub fn accumulated_ref(&self) -> Arc<RwLock<AccumulatedMetrics>> {
        self.accumulated.clone()
    }

    /// Genera un nuevo ID de ciclo
    pub fn next_cycle_id(&mut self) -> String {
        self.cycle_counter += 1;
        format!(
            "{}_cycle_{}",
            Local::now().format("%Y%m%d"),
            self.cycle_counter
        )
    }

    /// Registra métricas de un archivo en el log
    pub fn record_file_result(
        &self,
        result: &JobResult,
        cycle_id: &str,
        log_manager: &LogManager,
    ) {
        let metrics = FileMetrics {
            timestamp: Local::now().format("%Y-%m-%dT%H:%M:%S%.3f%:z").to_string(),
            cycle_id: cycle_id.to_string(),
            relative_path: result.relative_path.to_string_lossy().to_string(),
            source_path: String::new(), // Se rellena por el caller si es necesario
            destination_path: String::new(),
            file_size_bytes: result.bytes_copied,
            copy_duration_ms: result.copy_duration_ms,
            hash_duration_ms: result.hash_duration_ms,
            total_duration_ms: result.total_duration_ms,
            average_speed_mbps: result.average_speed_mbps,
            retry_count: result.retry_count,
            final_state: result.state.to_string(),
            error_message_if_any: result.error_message.clone().unwrap_or_default(),
        };

        if let Err(e) = log_manager.write_file_metrics(&metrics) {
            tracing::warn!("Error escribiendo métricas de archivo: {}", e);
        }
    }

    /// Genera y registra métricas de un ciclo completo
    pub fn record_cycle(
        &mut self,
        cycle_id: &str,
        results: &[JobResult],
        files_detected: u64,
        files_unstable: u64,
        cycle_start: Instant,
        log_manager: &LogManager,
    ) -> CycleMetrics {
        let duration = cycle_start.elapsed();

        let files_copied = results.iter().filter(|r| r.state == FileState::Finalized).count() as u64;
        let files_already = results.iter().filter(|r| r.state == FileState::AlreadyExistsSameHash).count() as u64;
        let files_versioned = results.iter().filter(|r| r.state == FileState::VersionedCopyCreated).count() as u64;
        let files_conflicted = results.iter().filter(|r| r.state == FileState::ConflictDestinationExistsDifferentHash).count() as u64;
        let files_failed = results.iter().filter(|r| r.state.is_error()).count() as u64;
        let bytes_copied: u64 = results.iter().map(|r| r.bytes_copied).sum();
        let files_stable = results.len() as u64;

        let duration_secs = duration.as_secs_f64();
        let avg_speed = if duration_secs > 0.0 {
            (bytes_copied as f64 / 1_048_576.0) / duration_secs
        } else {
            0.0
        };

        let peak_speed = results
            .iter()
            .map(|r| r.average_speed_mbps)
            .fold(0.0f64, f64::max);

        // Actualizar errores consecutivos
        if files_failed > 0 {
            self.consecutive_errors += files_failed as u32;
        } else {
            self.consecutive_errors = 0;
        }

        let cycle_metrics = CycleMetrics {
            timestamp: Local::now().format("%Y-%m-%dT%H:%M:%S%.3f%:z").to_string(),
            cycle_id: cycle_id.to_string(),
            files_detected,
            files_stable,
            files_unstable,
            files_copied,
            files_already_backed_up: files_already,
            files_versioned,
            files_conflicted,
            files_failed,
            bytes_copied,
            cycle_duration_seconds: duration_secs,
            average_speed_mbps: avg_speed,
            peak_speed_mbps: peak_speed,
            consecutive_errors: self.consecutive_errors,
        };

        // Actualizar métricas acumuladas
        {
            let mut acc = self.accumulated.write();
            acc.uptime_seconds = self.start_time.elapsed().as_secs_f64();
            acc.total_files_processed += results.len() as u64;
            acc.total_files_copied += files_copied;
            acc.total_files_failed += files_failed;
            acc.total_bytes_copied += bytes_copied;
            if acc.total_bytes_copied > 0 && acc.uptime_seconds > 0.0 {
                acc.global_average_speed_mbps =
                    (acc.total_bytes_copied as f64 / 1_048_576.0) / acc.uptime_seconds;
            }
        }

        // Registrar en log
        if let Err(e) = log_manager.write_cycle_metrics(&cycle_metrics) {
            tracing::warn!("Error escribiendo métricas de ciclo: {}", e);
        }

        debug!(
            "Ciclo {}: {} copiados, {} ya existentes, {} fallidos, {:.1} MB/s, {:.1}s",
            cycle_id, files_copied, files_already, files_failed, avg_speed, duration_secs
        );

        cycle_metrics
    }

    /// Actualiza el estado del NAS en métricas
    pub fn set_nas_status(&self, status: &str) {
        self.accumulated.write().current_nas_status = status.to_string();
    }

    /// Actualiza la profundidad de la cola
    pub fn set_queue_depth(&self, depth: u64) {
        self.accumulated.write().current_queue_depth = depth;
    }
}
