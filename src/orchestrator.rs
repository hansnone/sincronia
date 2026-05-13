// sincronia/src/orchestrator.rs
//
// Máquina de estados global: montaje DOS por par → escaneo → estabilidad
// → planificación → copia → desmontaje → siguiente par.

use crate::config::SincroniaConfig;
use crate::errors::GlobalState;
use crate::exclusions::ExclusionFilter;
use crate::logging::LogManager;
use crate::planner;
use crate::scanner;
use crate::scheduler::{WorkerConfig, WorkerPool};
use crate::shutdown::ShutdownSignal;
use crate::stability::StabilityChecker;
use crate::stats::StatsAggregator;
use crossbeam_channel::Sender;
use parking_lot::RwLock;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{error, info, warn};

#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
#[cfg(windows)]
use std::ptr::null;

#[cfg(windows)]
const DDD_REMOVE_DEFINITION: u32 = 0x00000002;

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn DefineDosDeviceW(
        dwflags: u32,
        lpdevicename: *const u16,
        lptargetpath: *const u16,
    ) -> i32;
}

/// Raíz de unidad virtual tipo `X:\` para escaneo y rutas absolutas coherentes.
fn virtual_drive_root(letter: &str) -> PathBuf {
    let l = letter.trim_end_matches(['\\', '/']);
    PathBuf::from(format!("{}\\", l))
}

#[cfg(windows)]
fn wide_device_name(device: &str) -> Vec<u16> {
    device.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn wide_target_path(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(windows)]
fn define_dos_device_remove(device_name: &str) {
    let dev = wide_device_name(device_name);
    let rc = unsafe { DefineDosDeviceW(DDD_REMOVE_DEFINITION, dev.as_ptr(), null()) };
    if rc == 0 {
        warn!(
            "DefineDosDeviceW(desmontar) {}: {}",
            device_name,
            std::io::Error::last_os_error()
        );
    }
}

#[cfg(windows)]
fn define_dos_device_define(device_name: &str, target_path: &Path) -> Result<(), String> {
    let dev = wide_device_name(device_name);
    let tgt = wide_target_path(target_path);
    let rc = unsafe { DefineDosDeviceW(0, dev.as_ptr(), tgt.as_ptr()) };
    if rc == 0 {
        Err(format!(
            "DefineDosDeviceW(montar) {} → '{}': {}",
            device_name,
            target_path.display(),
            std::io::Error::last_os_error()
        ))
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn define_dos_device_remove(_device_name: &str) {}

#[cfg(not(windows))]
fn define_dos_device_define(_device_name: &str, _target_path: &Path) -> Result<(), String> {
    Err("DefineDosDeviceW solo está disponible en Windows".into())
}

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

    fn unmount_pair_letters(&self, pair: &crate::config::SyncPairConfig) {
        define_dos_device_remove(pair.source_virtual_drive_letter.trim_end_matches(['\\', '/']));
        define_dos_device_remove(pair.target_virtual_drive_letter.trim_end_matches(['\\', '/']));
    }

    /// `true` si ambas letras quedaron montadas.
    fn mount_pair(&self, pair: &crate::config::SyncPairConfig) -> bool {
        let src_letter = pair
            .source_virtual_drive_letter
            .trim_end_matches(['\\', '/']);
        let tgt_letter = pair
            .target_virtual_drive_letter
            .trim_end_matches(['\\', '/']);

        define_dos_device_remove(src_letter);
        define_dos_device_remove(tgt_letter);

        if let Err(e) = define_dos_device_define(src_letter, &pair.source_path) {
            error!("No se pudo montar origen virtual {}: {}", src_letter, e);
            return false;
        }
        if let Err(e) = define_dos_device_define(tgt_letter, &pair.target_path) {
            error!("No se pudo montar destino virtual {}: {}", tgt_letter, e);
            define_dos_device_remove(src_letter);
            return false;
        }
        true
    }

    /// Ejecuta el loop principal del orquestador
    pub fn run(&mut self) {
        info!("═══ Sincronia Orchestrator iniciado ═══");
        self.set_state(GlobalState::LoadingConfiguration);

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

        let mut stability_checkers: Vec<StabilityChecker> = self
            .config
            .sync_pairs
            .iter()
            .map(|p| StabilityChecker::new(p.minimum_file_stable_seconds))
            .collect();

        let mut stats = StatsAggregator::new();

        self.set_state(GlobalState::ValidatingConfiguration);

        'global_loop: loop {
            if self.shutdown.is_shutdown_requested() {
                self.set_state(GlobalState::Stopping);
                info!("Parada ordenada solicitada — saliendo del loop principal");
                break;
            }

            if let Some(ref rx) = self.command_receiver {
                while let Ok(cmd) = rx.try_recv() {
                    match cmd {
                        TrayCommand::Pause => {
                            info!("Pausado por el usuario");
                            self.set_state(GlobalState::Paused);
                        }
                        TrayCommand::Resume => {
                            info!("Reanudado por el usuario");
                            self.set_state(GlobalState::Idle);
                        }
                        TrayCommand::Stop => {
                            info!("Parada solicitada desde bandeja");
                            self.shutdown.trigger();
                        }
                    }
                }
            }

            if *self.state.read() == GlobalState::Paused {
                std::thread::sleep(Duration::from_secs(1));
                continue;
            }

            let mut had_stable_files_any_pair = false;
            let mut extended_pause = false;

            for (pair_index, pair) in self.config.sync_pairs.iter().enumerate() {
                if self.shutdown.is_shutdown_requested() {
                    break;
                }

                self.unmount_pair_letters(pair);

                if !self.mount_pair(pair) {
                    warn!(
                        "Omitiendo par {}: no se pudieron montar unidades virtuales",
                        pair_index
                    );
                    self.unmount_pair_letters(pair);
                    continue;
                }

                let pair_detail = format!(
                    "{} → {}",
                    pair.source_path.display(),
                    pair.target_path.display()
                );

                let src_root = virtual_drive_root(&pair.source_virtual_drive_letter);
                let dest_base_path = virtual_drive_root(&pair.target_virtual_drive_letter);

                self.set_state(GlobalState::Scanning);
                let scan_result = scanner::scan_directory(
                    &src_root,
                    &filter,
                    self.config.copy_engine.ignore_symbolic_links,
                    self.config.copy_engine.ignore_junctions,
                );

                let files_detected = scan_result.files.len() as u64;

                if scan_result.files.is_empty() {
                    self.unmount_pair_letters(pair);
                    continue;
                }

                let (stable_files, unstable_files) =
                    stability_checkers[pair_index].evaluate(&scan_result.files);
                let files_unstable = unstable_files.len() as u64;
                stats.set_queue_depth(stable_files.len() as u64);

                if stable_files.is_empty() {
                    self.unmount_pair_letters(pair);
                    continue;
                }

                had_stable_files_any_pair = true;

                let large_threshold_bytes =
                    self.config.copy_engine.large_file_threshold_mib * 1024 * 1024;

                let jobs = planner::plan_copy_jobs(
                    &stable_files,
                    &dest_base_path,
                    &self.config.copy_engine.temporary_destination_extension,
                    large_threshold_bytes,
                );

                if self.config.copy_engine.preserve_empty_directory_hierarchy {
                    if let Err(e) = planner::ensure_destination_directories(
                        &scan_result.directories,
                        &dest_base_path,
                    ) {
                        error!("Error creando directorios en destino: {}", e);
                    }
                }

                self.set_state(GlobalState::Copying(pair_detail.clone()));
                let cycle_id = stats.next_cycle_id();
                let cycle_start = Instant::now();

                info!(
                    "═══ Par {} — Ciclo {} — {} archivos estables, {} trabajos ═══",
                    pair_index,
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

                let mut pool = WorkerPool::new(
                    self.config.copy_engine.worker_count,
                    self.config.copy_engine.copy_buffer_size_mib_per_worker,
                    worker_config,
                    self.shutdown.as_atomic(),
                );

                for job in jobs {
                    if self.shutdown.is_shutdown_requested() {
                        break;
                    }
                    if let Err(e) = pool.submit(job) {
                        error!("Error enviando trabajo al pool: {}", e);
                    }
                }

                let results = pool.wait_for_completion(Duration::from_secs(3600));

                let entry_lookup: std::collections::HashMap<_, _> = stable_files
                    .iter()
                    .map(|e| (e.relative_path.clone(), e))
                    .collect();

                let checker = &mut stability_checkers[pair_index];
                for result in &results {
                    stats.record_file_result(result, &cycle_id, &log_manager);

                    if result.state.is_success() {
                        if result.state == crate::errors::FileState::AlreadyExistsSameHash {
                            if let Some(entry) = entry_lookup.get(&result.relative_path) {
                                checker.mark_backed_up(entry);
                            }
                        } else {
                            checker.mark_processed(&result.relative_path);
                        }
                    }
                }

                let cycle_metrics = stats.record_cycle(
                    &cycle_id,
                    &results,
                    files_detected,
                    files_unstable,
                    cycle_start,
                    &log_manager,
                );

                if let Some(ref sender) = self.tray_sender {
                    sender
                        .send(OrchestratorMessage::CycleCompleted {
                            files_copied: cycle_metrics.files_copied,
                            bytes_copied: cycle_metrics.bytes_copied,
                        })
                        .ok();
                }

                if cycle_metrics.files_failed > 0 {
                    self.notify(
                        "Sincronia — Errores en ciclo",
                        &format!(
                            "{} archivos con error en ciclo {} (par {})",
                            cycle_metrics.files_failed, cycle_id, pair_index
                        ),
                    );
                }

                pool.shutdown();

                if self
                    .config
                    .copy_engine
                    .remove_empty_source_directories_after_successful_processing
                {
                    planner::remove_empty_source_directories(&src_root, &scan_result.directories);
                }

                self.unmount_pair_letters(pair);

                if stats.consecutive_errors
                    >= self
                        .config
                        .retry_policy
                        .maximum_consecutive_errors_before_extended_pause
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
                    extended_pause = true;
                    break;
                }
            }

            if self.shutdown.is_shutdown_requested() {
                break 'global_loop;
            }

            if extended_pause {
                continue 'global_loop;
            }

            self.set_state(GlobalState::Idle);
            let interval_secs = if had_stable_files_any_pair {
                self.config
                    .scan
                    .scan_interval_seconds_after_changes
            } else {
                self.config
                    .scan
                    .scan_interval_seconds_when_no_changes
            };
            self.interruptible_sleep(Duration::from_secs(interval_secs));
        }

        for p in &self.config.sync_pairs {
            self.unmount_pair_letters(p);
        }

        self.set_state(GlobalState::Stopped);
        info!("═══ Sincronia Orchestrator detenido ═══");
    }

    /// Sleep interruptible por señal de parada
    fn interruptible_sleep(&self, duration: Duration) {
        let start = Instant::now();
        while start.elapsed() < duration {
            if self.shutdown.is_shutdown_requested() {
                return;
            }
            if let Some(ref rx) = self.command_receiver {
                if let Ok(cmd) = rx.try_recv() {
                    match cmd {
                        TrayCommand::Stop => {
                            self.shutdown.trigger();
                            return;
                        }
                        TrayCommand::Resume => {
                            return;
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
