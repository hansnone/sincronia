// sincronia/src/orchestrator.rs
//
// Máquina de estados global que coordina todo el ciclo:
// NAS → Scan → Estabilidad → Planificación → Copia → Métricas → Espera

use crate::config::SincroniaConfig;
use crate::credentials;
use crate::errors::GlobalState;
use crate::exclusions::ExclusionFilter;
use crate::logging::LogManager;
use crate::planner;
use crate::scanner;
use crate::scheduler::{WorkerConfig, WorkerPool};
use crate::shutdown::ShutdownSignal;
use crate::stability::StabilityChecker;
use crate::stats::StatsAggregator;
use crate::windows_nas::{self, DriveValidation};
use crossbeam_channel::Sender;
use parking_lot::RwLock;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{error, info, warn};

/// Mensajes del orquestador al tray
#[derive(Debug, Clone)]
pub enum OrchestratorMessage {
    StateChanged(GlobalState),
    Notification { title: String, message: String },
    CycleCompleted { files_copied: u64, bytes_copied: u64 },
}

/// Comandos del tray al orquestador
#[derive(Debug, Clone)]
pub enum TrayCommand {
    Pause,
    Resume,
    MountNas,
    RetryConnection,
    Stop,
}

/// Estado compartido del orquestador (lectura desde tray)
pub struct OrchestratorState {
    pub global_state: Arc<RwLock<GlobalState>>,
}

/// Orquestador principal del sistema
pub struct Orchestrator {
    config: SincroniaConfig,
    state: Arc<RwLock<GlobalState>>,
    shutdown: ShutdownSignal,
    tray_sender: Option<Sender<OrchestratorMessage>>,
    command_receiver: Option<crossbeam_channel::Receiver<TrayCommand>>,
}

impl Orchestrator {
    pub fn new(
        config: SincroniaConfig,
        shutdown: ShutdownSignal,
        tray_sender: Option<Sender<OrchestratorMessage>>,
        command_receiver: Option<crossbeam_channel::Receiver<TrayCommand>>,
    ) -> Self {
        Self {
            config,
            state: Arc::new(RwLock::new(GlobalState::Starting)),
            shutdown,
            tray_sender,
            command_receiver,
        }
    }

    /// Referencia compartida al estado global
    pub fn state_ref(&self) -> Arc<RwLock<GlobalState>> {
        self.state.clone()
    }

    fn set_state(&self, new_state: GlobalState) {
        info!("Estado global: {} → {}", *self.state.read(), &new_state);
        *self.state.write() = new_state.clone();
        if let Some(ref sender) = self.tray_sender {
            sender
                .send(OrchestratorMessage::StateChanged(new_state))
                .ok();
        }
    }

    fn notify(&self, title: &str, message: &str) {
        if let Some(ref sender) = self.tray_sender {
            sender
                .send(OrchestratorMessage::Notification {
                    title: title.to_string(),
                    message: message.to_string(),
                })
                .ok();
        }
    }

    /// Ejecuta el loop principal del orquestador
    pub fn run(&mut self) {
        info!("═══ Sincronia Orchestrator iniciado ═══");
        self.set_state(GlobalState::LoadingConfiguration);

        // Crear componentes
        let filter = ExclusionFilter::new(
            &self.config.exclusions.excluded_directory_names,
            &self.config.exclusions.excluded_file_patterns,
        );

        let log_manager = match LogManager::init(
            &self.config.logging.human_log_file_path,
            &self.config.logging.metrics_csv_file_path,
            &self.config.logging.events_jsonl_file_path,
        ) {
            Ok(lm) => lm,
            Err(e) => {
                error!("Error fatal al inicializar logging: {}", e);
                self.set_state(GlobalState::ErrorPersistent);
                return;
            }
        };

        let mut stability = StabilityChecker::new(self.config.source.minimum_file_stable_seconds);
        let mut stats = StatsAggregator::new();

        self.set_state(GlobalState::ValidatingConfiguration);

        // ── Loop principal ──
        loop {
            if self.shutdown.is_shutdown_requested() {
                self.set_state(GlobalState::Stopping);
                info!("Parada ordenada solicitada — saliendo del loop principal");
                break;
            }

            // Procesar comandos del tray
            if let Some(ref rx) = self.command_receiver {
                while let Ok(cmd) = rx.try_recv() {
                    match cmd {
                        TrayCommand::Pause => {
                            info!("Pausado por el usuario");
                            self.set_state(GlobalState::Paused);
                        }
                        TrayCommand::Resume => {
                            info!("Reanudado por el usuario");
                            self.set_state(GlobalState::NasAvailable);
                        }
                        TrayCommand::Stop => {
                            info!("Parada solicitada desde bandeja");
                            self.shutdown.trigger();
                        }
                        TrayCommand::MountNas | TrayCommand::RetryConnection => {
                            info!("Reintento de conexión NAS solicitado");
                            // Se intentará en la siguiente iteración del loop
                        }
                    }
                }
            }

            // Si está pausado, esperar
            if *self.state.read() == GlobalState::Paused {
                std::thread::sleep(Duration::from_secs(1));
                continue;
            }

            // 1. Validar NAS
            self.set_state(GlobalState::CheckingNasMapping);
            if !self.ensure_nas_available(&mut stats) {
                let delay = self.config.retry_policy.nas_retry_delay_seconds;
                warn!("NAS no disponible — esperando {}s", delay);
                self.interruptible_sleep(Duration::from_secs(delay));
                continue;
            }

            stats.set_nas_status("available");
            self.set_state(GlobalState::NasAvailable);

            // 2. Escanear directorio origen
            self.set_state(GlobalState::Scanning);
            let scan_result = scanner::scan_directory(
                &self.config.source.source_directory_path,
                &filter,
                self.config.copy_engine.ignore_symbolic_links,
                self.config.copy_engine.ignore_junctions,
            );

            let files_detected = scan_result.files.len() as u64;

            if scan_result.files.is_empty() {
                self.set_state(GlobalState::Idle);
                let interval = self.config.source.scan_interval_seconds_when_no_changes;
                self.interruptible_sleep(Duration::from_secs(interval));
                continue;
            }

            // 3. Evaluar estabilidad
            let (stable_files, unstable_files) = stability.evaluate(&scan_result.files);
            let files_unstable = unstable_files.len() as u64;
            stats.set_queue_depth(stable_files.len() as u64);

            if stable_files.is_empty() {
                self.set_state(GlobalState::Idle);
                let interval = self.config.source.scan_interval_seconds_after_changes;
                self.interruptible_sleep(Duration::from_secs(interval));
                continue;
            }

            // 4. Planificar trabajos
            let dest_base_path = std::path::Path::new(&self.config.nas.required_drive_letter);
            let large_threshold_bytes =
                self.config.copy_engine.large_file_threshold_mib * 1024 * 1024;

            let jobs = planner::plan_copy_jobs(
                &stable_files,
                dest_base_path,
                &self.config.copy_engine.temporary_destination_extension,
                large_threshold_bytes,
            );

            // Crear directorios en destino
            if self.config.copy_engine.preserve_empty_directory_hierarchy {
                if let Err(e) = planner::ensure_destination_directories(
                    &scan_result.directories,
                    dest_base_path,
                ) {
                    error!("Error creando directorios en destino: {}", e);
                }
            }

            // 5. Ejecutar copia con worker pool
            self.set_state(GlobalState::Copying);
            let cycle_id = stats.next_cycle_id();
            let cycle_start = Instant::now();

            info!(
                "═══ Ciclo {} — {} archivos estables, {} trabajos ═══",
                cycle_id,
                stable_files.len(),
                jobs.len()
            );

            let worker_config = WorkerConfig {
                copy_engine: self.config.copy_engine.clone(),
                verification: self.config.verification.clone(),
                metadata: self.config.metadata.clone(),
                conflicts: self.config.conflicts.clone(),
                hash_algorithm: self.config.verification.hash_algorithm.clone(),
                run_mode: self.config.general.run_mode.clone(),
                retries_per_file: self.config.retry_policy.retries_per_file,
                retry_delays: self.config.retry_policy.retry_delay_seconds_sequence.clone(),
            };

            let pool = WorkerPool::new(
                self.config.copy_engine.worker_count,
                self.config.copy_engine.copy_buffer_size_mib_per_worker,
                worker_config,
                self.shutdown.as_atomic(),
            );

            // Enviar trabajos
            for job in jobs {
                if self.shutdown.is_shutdown_requested() {
                    break;
                }
                if let Err(e) = pool.submit(job) {
                    error!("Error enviando trabajo al pool: {}", e);
                }
            }

            // Esperar resultados
            let results = pool.wait_for_completion(Duration::from_secs(3600));

            // Crear mapa de lookup para FileEntry por ruta relativa
            let entry_lookup: std::collections::HashMap<_, _> = stable_files
                .iter()
                .map(|e| (e.relative_path.clone(), e))
                .collect();

            // Registrar métricas por archivo
            for result in &results {
                stats.record_file_result(result, &cycle_id, &log_manager);

                // Marcar archivos procesados exitosamente
                if result.state.is_success() {
                    if result.state == crate::errors::FileState::AlreadyExistsSameHash {
                        // Archivo idéntico ya existe en destino → cachear para
                        // evitar re-hasheo en futuros ciclos
                        if let Some(entry) = entry_lookup.get(&result.relative_path) {
                            stability.mark_backed_up(entry);
                        }
                    } else {
                        stability.mark_processed(&result.relative_path);
                    }
                }
            }

            // Registrar métricas del ciclo
            let cycle_metrics = stats.record_cycle(
                &cycle_id,
                &results,
                files_detected,
                files_unstable,
                cycle_start,
                &log_manager,
            );

            // Notificar al tray
            if let Some(ref sender) = self.tray_sender {
                sender
                    .send(OrchestratorMessage::CycleCompleted {
                        files_copied: cycle_metrics.files_copied,
                        bytes_copied: cycle_metrics.bytes_copied,
                    })
                    .ok();
            }

            // Notificar si hubo errores
            if cycle_metrics.files_failed > 0 {
                self.notify(
                    "Sincronia — Errores en ciclo",
                    &format!(
                        "{} archivos con error en ciclo {}",
                        cycle_metrics.files_failed, cycle_id
                    ),
                );
            }

            // Detener el pool
            pool.shutdown();

            // 6. Limpiar directorios vacíos del origen
            if self.config.copy_engine.remove_empty_source_directories_after_successful_processing {
                planner::remove_empty_source_directories(
                    &self.config.source.source_directory_path,
                    &scan_result.directories,
                );
            }

            // 7. Verificar errores consecutivos
            if stats.consecutive_errors
                >= self.config.retry_policy.maximum_consecutive_errors_before_extended_pause
            {
                self.set_state(GlobalState::ErrorPersistent);
                let delay = self.config.retry_policy.persistent_error_delay_seconds;
                warn!(
                    "{} errores consecutivos — pausa extendida de {}s",
                    stats.consecutive_errors, delay
                );
                self.notify(
                    "Sincronia — Error persistente",
                    &format!(
                        "{} errores consecutivos. Pausa de {} segundos.",
                        stats.consecutive_errors, delay
                    ),
                );
                self.interruptible_sleep(Duration::from_secs(delay));
                continue;
            }

            // Esperar antes del siguiente ciclo
            self.set_state(GlobalState::Idle);
            let interval = self.config.source.scan_interval_seconds_after_changes;
            self.interruptible_sleep(Duration::from_secs(interval));
        }

        self.set_state(GlobalState::Stopped);
        info!("═══ Sincronia Orchestrator detenido ═══");
    }

    /// Asegura que el NAS esté disponible
    fn ensure_nas_available(&self, stats: &mut StatsAggregator) -> bool {
        match windows_nas::validate_drive(&self.config.nas) {
            DriveValidation::ValidPrimary | DriveValidation::ValidFallbackIp => true,
            DriveValidation::NotMounted => {
                info!("NAS no montado — intentando montar...");
                // Intentar montar sin credenciales primero (Kerberos)
                match windows_nas::attempt_mount(&self.config.nas, None, None) {
                    Ok(()) => {
                        info!("NAS montado correctamente vía Kerberos");
                        true
                    }
                    Err(_) => {
                        // Necesita credenciales — mostrar diálogo nativo de Windows
                        self.set_state(GlobalState::WaitingForCredentials);
                        self.notify(
                            "Sincronia — Credenciales requeridas",
                            "Se necesitan credenciales para conectar al NAS.",
                        );

                        match credentials::prompt_credentials_gui(
                            &self.config.nas.primary_unc_path,
                            self.config.nas.maximum_credential_prompt_attempts,
                        ) {
                            Ok(creds) => {
                                match windows_nas::attempt_mount(
                                    &self.config.nas,
                                    Some(&creds.username),
                                    Some(&creds.password),
                                ) {
                                    Ok(()) => {
                                        info!("NAS montado con credenciales");
                                        true
                                    }
                                    Err(e) => {
                                        error!("Fallo al montar NAS con credenciales: {}", e);
                                        self.set_state(GlobalState::ErrorPersistent);
                                        stats.set_nas_status("mount_failed");
                                        false
                                    }
                                }
                            }
                            Err(e) => {
                                warn!("No se obtuvieron credenciales: {}", e);
                                self.set_state(GlobalState::ErrorTransient);
                                stats.set_nas_status("credentials_unavailable");
                                false
                            }
                        }
                    }
                }
            }
            DriveValidation::PointsElsewhere { current_target } => {
                if self.config.nas.allow_automatic_remap_if_drive_points_elsewhere {
                    warn!(
                        "R: apunta a '{}' — intentando remapear",
                        current_target
                    );
                    if let Err(e) = windows_nas::unmount_drive(&self.config.nas.required_drive_letter) {
                        error!("Error al desmontar R: para remapeo: {}", e);
                        return false;
                    }
                    match windows_nas::attempt_mount(&self.config.nas, None, None) {
                        Ok(()) => true,
                        Err(e) => {
                            error!("Fallo al remontar R:: {}", e);
                            false
                        }
                    }
                } else {
                    error!(
                        "R: apunta a '{}' en lugar del NAS. Remapeo automático deshabilitado.",
                        current_target
                    );
                    self.set_state(GlobalState::ErrorPersistent);
                    self.notify(
                        "Sincronia — R: incorrecta",
                        &format!(
                            "R: apunta a '{}'. Desmonte manualmente y reconecte.",
                            current_target
                        ),
                    );
                    stats.set_nas_status("drive_mismatch");
                    false
                }
            }
            DriveValidation::Error(msg) => {
                error!("Error al validar NAS: {}", msg);
                self.set_state(GlobalState::ErrorTransient);
                stats.set_nas_status("validation_error");
                false
            }
        }
    }

    /// Sleep interruptible por señal de parada
    fn interruptible_sleep(&self, duration: Duration) {
        let start = Instant::now();
        while start.elapsed() < duration {
            if self.shutdown.is_shutdown_requested() {
                return;
            }
            // Procesar comandos del tray durante el sleep
            if let Some(ref rx) = self.command_receiver {
                if let Ok(cmd) = rx.try_recv() {
                    match cmd {
                        TrayCommand::Stop => {
                            self.shutdown.trigger();
                            return;
                        }
                        TrayCommand::Resume | TrayCommand::RetryConnection | TrayCommand::MountNas => {
                            return; // Salir del sleep para actuar
                        }
                        TrayCommand::Pause => {
                            self.set_state(GlobalState::Paused);
                        }
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }
}
