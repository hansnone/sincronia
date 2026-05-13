// sincronia/src/scheduler.rs
//
// Pool de workers con canal crossbeam para procesamiento concurrente.
// Scheduler adaptativo: reduce workers activos para archivos grandes.
// Cada worker tiene buffer preasignado para evitar allocations.

use crate::config::{CopyEngineConfig, MetadataConfig, VerificationConfig, ConflictConfig};
use crate::conflict::{self, ConflictResolution};
use crate::copy_engine::{self, CopyResult};
use crate::errors::{FileState, HashAlgorithm, SincroniaError, RunMode};
use crate::metadata;
use crate::planner::CopyJob;
use crate::verifier;
use crossbeam_channel::{Receiver, Sender};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};

/// Resultado del procesamiento de un trabajo
#[derive(Debug, Clone)]
pub struct JobResult {
    /// Ruta relativa del archivo
    pub relative_path: std::path::PathBuf,
    /// Estado final del archivo
    pub state: FileState,
    /// Bytes copiados (0 si no se copió)
    pub bytes_copied: u64,
    /// Duración de la copia en ms
    pub copy_duration_ms: u64,
    /// Duración del hash en ms
    pub hash_duration_ms: u64,
    /// Duración total en ms
    pub total_duration_ms: u64,
    /// Velocidad media MB/s
    pub average_speed_mbps: f64,
    /// Número de reintentos realizados
    pub retry_count: u32,
    /// Mensaje de error (si hubo)
    pub error_message: Option<String>,
}

/// Configuración pasada a los workers
#[derive(Clone)]
pub struct WorkerConfig {
    pub copy_engine: CopyEngineConfig,
    pub verification: VerificationConfig,
    pub metadata: MetadataConfig,
    pub conflicts: ConflictConfig,
    pub hash_algorithm: HashAlgorithm,
    pub run_mode: RunMode,
    pub retries_per_file: u32,
    pub retry_delays: Vec<u64>,
}

/// Pool de workers para procesamiento de trabajos de copia
pub struct WorkerPool {
    /// Sender para enviar trabajos a los workers
    job_sender: Sender<CopyJob>,
    /// Receiver para recoger resultados
    result_receiver: Receiver<JobResult>,
    /// Handles de los threads workers
    worker_handles: Vec<thread::JoinHandle<()>>,
    /// Señal de parada
    shutdown: Arc<AtomicBool>,
    /// Contador de trabajos enviados (para saber cuántos resultados esperar)
    submitted_count: Arc<AtomicUsize>,
}

impl WorkerPool {
    /// Crea el pool de workers
    pub fn new(
        worker_count: usize,
        buffer_size_mib: usize,
        worker_config: WorkerConfig,
        shutdown_signal: Arc<AtomicBool>,
    ) -> Self {
        let (job_sender, job_receiver) = crossbeam_channel::bounded::<CopyJob>(worker_count * 2);
        let (result_sender, result_receiver) = crossbeam_channel::unbounded::<JobResult>();

        let mut worker_handles = Vec::with_capacity(worker_count);
        let buffer_size = buffer_size_mib * 1024 * 1024;

        for worker_id in 0..worker_count {
            let rx = job_receiver.clone();
            let tx = result_sender.clone();
            let config = worker_config.clone();
            let shutdown = shutdown_signal.clone();

            let handle = thread::Builder::new()
                .name(format!("sincronia-worker-{}", worker_id))
                .spawn(move || {
                    // Buffer preasignado por worker — se reutiliza entre archivos
                    let mut buffer = vec![0u8; buffer_size];

                    debug!("Worker {} iniciado (buffer: {} MiB)", worker_id, buffer_size_mib);

                    while let Ok(job) = rx.recv() {
                        if shutdown.load(Ordering::Relaxed) {
                            debug!("Worker {} recibió señal de parada", worker_id);
                            break;
                        }

                        let result = process_job(&job, &mut buffer, &config, &shutdown);
                        if tx.send(result).is_err() {
                            break; // Canal cerrado
                        }
                    }

                    debug!("Worker {} terminado", worker_id);
                })
                .expect("Error al crear thread worker");

            worker_handles.push(handle);
        }

        info!("{} workers iniciados con buffer de {} MiB cada uno", worker_count, buffer_size_mib);

        Self {
            job_sender,
            result_receiver,
            worker_handles,
            shutdown: shutdown_signal,
            submitted_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Envía un trabajo al pool
    pub fn submit(&self, job: CopyJob) -> Result<(), SincroniaError> {
        self.job_sender.send(job).map_err(|_| SincroniaError::Copy {
            path: std::path::PathBuf::new(),
            message: "Canal de trabajos cerrado".into(),
        })?;
        self.submitted_count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    /// Recoge todos los resultados disponibles (no bloqueante)
    pub fn collect_results(&self) -> Vec<JobResult> {
        let mut results = Vec::new();
        while let Ok(result) = self.result_receiver.try_recv() {
            results.push(result);
        }
        results
    }

    /// Espera a que todos los trabajos pendientes se completen.
    /// Espera hasta recibir exactamente tantos resultados como trabajos enviados,
    /// o hasta que se agote el timeout.
    pub fn wait_for_completion(&self, timeout: Duration) -> Vec<JobResult> {
        let expected = self.submitted_count.load(Ordering::SeqCst);
        let mut results = Vec::with_capacity(expected);
        let start = Instant::now();

        while results.len() < expected && start.elapsed() < timeout {
            if self.shutdown.load(Ordering::Relaxed) {
                // En shutdown, recoger lo que haya disponible sin bloquear
                while let Ok(result) = self.result_receiver.try_recv() {
                    results.push(result);
                }
                break;
            }

            match self.result_receiver.recv_timeout(Duration::from_millis(500)) {
                Ok(result) => results.push(result),
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                    // Seguir esperando — hay workers procesando
                    debug!(
                        "Esperando resultados: {}/{} recibidos ({:.1}s)",
                        results.len(),
                        expected,
                        start.elapsed().as_secs_f64()
                    );
                }
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                    warn!(
                        "Canal de resultados desconectado con {}/{} resultados",
                        results.len(),
                        expected
                    );
                    break;
                }
            }
        }

        if results.len() < expected {
            warn!(
                "wait_for_completion terminó con {}/{} resultados (timeout={}, shutdown={})",
                results.len(),
                expected,
                start.elapsed() >= timeout,
                self.shutdown.load(Ordering::Relaxed)
            );
        }

        results
    }

    /// Detiene el pool de workers de forma ordenada
    pub fn shutdown(self) {
        self.shutdown.store(true, Ordering::Relaxed);
        drop(self.job_sender); // Cerrar canal para desbloquear recv()

        for handle in self.worker_handles {
            handle.join().ok();
        }
        info!("Pool de workers detenido");
    }
}

/// Procesa un trabajo de copia individual con reintentos
fn process_job(
    job: &CopyJob,
    buffer: &mut Vec<u8>,
    config: &WorkerConfig,
    shutdown: &Arc<AtomicBool>,
) -> JobResult {
    let start = Instant::now();
    let mut retry_count: u32 = 0;
    let mut last_error: Option<String> = None;

    for attempt in 0..=config.retries_per_file {
        if shutdown.load(Ordering::Relaxed) {
            return JobResult {
                relative_path: job.relative_path.clone(),
                state: FileState::SkippedAfterRetries,
                bytes_copied: 0,
                copy_duration_ms: 0,
                hash_duration_ms: 0,
                total_duration_ms: start.elapsed().as_millis() as u64,
                average_speed_mbps: 0.0,
                retry_count,
                error_message: Some("Parada ordenada".into()),
            };
        }

        if attempt > 0 {
            retry_count = attempt;
            let delay_idx = (attempt as usize - 1).min(config.retry_delays.len() - 1);
            let delay = config.retry_delays[delay_idx];
            warn!(
                "Reintento {}/{} para {} (esperando {}s)",
                attempt,
                config.retries_per_file,
                job.relative_path.display(),
                delay
            );
            thread::sleep(Duration::from_secs(delay));
        }

        match process_job_attempt(job, buffer, config) {
            Ok(result) => return result,
            Err(e) => {
                last_error = Some(e.to_string());
                error!(
                    "Error en intento {} para {}: {}",
                    attempt + 1,
                    job.relative_path.display(),
                    e
                );
                // Limpiar temporal si quedó un archivo parcial
                copy_engine::cleanup_temp_file(&job.temp_destination_path);
            }
        }
    }

    // Todos los reintentos agotados
    JobResult {
        relative_path: job.relative_path.clone(),
        state: FileState::SkippedAfterRetries,
        bytes_copied: 0,
        copy_duration_ms: 0,
        hash_duration_ms: 0,
        total_duration_ms: start.elapsed().as_millis() as u64,
        average_speed_mbps: 0.0,
        retry_count,
        error_message: last_error,
    }
}

/// Un intento individual de procesar un trabajo
fn process_job_attempt(
    job: &CopyJob,
    buffer: &mut Vec<u8>,
    config: &WorkerConfig,
) -> Result<JobResult, SincroniaError> {
    let start = Instant::now();

    // 1. Verificar conflictos — ¿destino ya existe?
    let (resolution, state) = conflict::resolve_conflict(
        &job.source_path,
        &job.destination_path,
        &config.conflicts,
        &config.hash_algorithm,
    )?;

    match resolution {
        ConflictResolution::AlreadyExists => {
            return Ok(JobResult {
                relative_path: job.relative_path.clone(),
                state,
                bytes_copied: 0,
                copy_duration_ms: 0,
                hash_duration_ms: 0,
                total_duration_ms: start.elapsed().as_millis() as u64,
                average_speed_mbps: 0.0,
                retry_count: 0,
                error_message: None,
            });
        }
        ConflictResolution::CreateVersionedCopy(versioned_path) => {
            // Copiar a la ruta versionada en lugar de la original
            let mut versioned_job = job.clone();
            versioned_job.destination_path = versioned_path.clone();
            versioned_job.temp_destination_path = std::path::PathBuf::from(format!(
                "{}{}",
                versioned_path.to_string_lossy(),
                config.copy_engine.temporary_destination_extension
            ));
            return execute_copy_pipeline(&versioned_job, buffer, config, start);
        }
        ConflictResolution::NoConflict => {
            // Continuar con la copia normal
        }
        ConflictResolution::Error(msg) => {
            return Err(SincroniaError::Conflict {
                path: job.destination_path.clone(),
                message: msg,
            });
        }
    }

    execute_copy_pipeline(job, buffer, config, start)
}

/// Ejecuta el pipeline completo: copiar → verificar → metadatos → finalizar
fn execute_copy_pipeline(
    job: &CopyJob,
    buffer: &mut Vec<u8>,
    config: &WorkerConfig,
    start: Instant,
) -> Result<JobResult, SincroniaError> {
    // 2. Copiar a archivo temporal
    let copy_result = copy_engine::copy_file_buffered(job, buffer)?;

    // 3. Verificar hash
    let verify_result = verifier::verify_copy(
        &job.source_path,
        &job.temp_destination_path,
        &config.verification,
    )?;

    if !verify_result.success {
        copy_engine::cleanup_temp_file(&job.temp_destination_path);
        return Err(SincroniaError::Hash {
            path: job.source_path.clone(),
            message: format!(
                "Hash no coincide: origen={}, destino={}",
                &verify_result.source_hash[..16.min(verify_result.source_hash.len())],
                &verify_result.destination_hash[..16.min(verify_result.destination_hash.len())]
            ),
        });
    }

    // 4. Aplicar metadatos al archivo temporal (antes de renombrar)
    metadata::apply_metadata(
        &job.temp_destination_path,
        &job.original_entry,
        &config.metadata,
    )?;

    // 5. Renombrar temporal → final
    copy_engine::finalize_copy(&job.temp_destination_path, &job.destination_path)?;

    // 6. Si es modo move, eliminar origen
    if config.run_mode == RunMode::MoveAfterVerifiedCopy
        && config.copy_engine.delete_source_after_verified_copy
    {
        match std::fs::remove_file(&job.source_path) {
            Ok(()) => info!("Origen eliminado tras copia verificada: {}", job.source_path.display()),
            Err(e) => warn!(
                "No se pudo eliminar origen '{}': {}",
                job.source_path.display(),
                e
            ),
        }
    }

    let total_ms = start.elapsed().as_millis() as u64;

    Ok(JobResult {
        relative_path: job.relative_path.clone(),
        state: FileState::Finalized,
        bytes_copied: copy_result.bytes_copied,
        copy_duration_ms: copy_result.copy_duration_ms,
        hash_duration_ms: verify_result.hash_duration_ms,
        total_duration_ms: total_ms,
        average_speed_mbps: copy_result.average_speed_mbps,
        retry_count: 0,
        error_message: None,
    })
}
