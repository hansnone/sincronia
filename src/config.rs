// sincronia/src/config.rs
//
// Configuración completa del sistema cargada desde archivo TOML.
// Todos los nombres de parámetros son largos y descriptivos.

use crate::errors::{
    ConflictExistsAction, HashAlgorithm, HashDifferentAction, HashEqualAction, RunMode,
    VerificationMode, VersionedNamingStrategy,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ─────────────────────────────────────────────
// Configuración raíz
// ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SincroniaConfig {
    pub general: GeneralConfig,
    #[serde(default)]
    pub scan: ScanTimingConfig,
    pub sync_pairs: Vec<SyncPairConfig>,
    pub copy_engine: CopyEngineConfig,
    pub verification: VerificationConfig,
    pub metadata: MetadataConfig,
    pub conflicts: ConflictConfig,
    pub exclusions: ExclusionConfig,
    pub retry_policy: RetryPolicyConfig,
    pub logging: LoggingConfig,
    pub scheduled_task: ScheduledTaskConfig,
}

// ─────────────────────────────────────────────
// Secciones individuales
// ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralConfig {
    /// Nombre visible de la aplicación
    #[serde(default = "default_application_name")]
    pub application_name: String,

    /// Modo de ejecución: backup_append_only o move_after_verified_copy
    #[serde(default)]
    pub run_mode: RunMode,

    /// Idioma para mensajes (reservado para futuro)
    #[serde(default = "default_language")]
    pub language: String,
}

/// Intervalos entre vueltas completas de sincronización (todos los pares).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanTimingConfig {
    /// Intervalo de escaneo en segundos cuando no hubo trabajo estable en la vuelta
    #[serde(default = "default_scan_no_changes")]
    pub scan_interval_seconds_when_no_changes: u64,

    /// Intervalo en segundos tras una vuelta donde al menos un par tuvo archivos estables
    #[serde(default = "default_scan_after_changes")]
    pub scan_interval_seconds_after_changes: u64,
}

impl Default for ScanTimingConfig {
    fn default() -> Self {
        Self {
            scan_interval_seconds_when_no_changes: default_scan_no_changes(),
            scan_interval_seconds_after_changes: default_scan_after_changes(),
        }
    }
}

/// Un par origen/destino con letras DOS virtuales para la sesión de copia.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPairConfig {
    /// Ruta real del directorio origen (DefineDosDevice apunta la letra virtual aquí)
    pub source_path: PathBuf,

    /// Letra virtual de origen (ej: "X:")
    pub source_virtual_drive_letter: String,

    /// Ruta real del directorio destino
    pub target_path: PathBuf,

    /// Letra virtual de destino (ej: "Y:")
    pub target_virtual_drive_letter: String,

    /// Segundos que un archivo debe permanecer sin cambios para considerarse estable
    #[serde(default = "default_stable_seconds")]
    pub minimum_file_stable_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopyEngineConfig {
    /// Número de workers de copia concurrentes
    #[serde(default = "default_worker_count")]
    pub worker_count: usize,

    /// Número máximo de workers permitidos
    #[serde(default = "default_max_workers")]
    pub maximum_worker_count: usize,

    /// Tamaño del buffer de copia por worker en MiB
    #[serde(default = "default_buffer_mib")]
    pub copy_buffer_size_mib_per_worker: usize,

    /// Umbral en MiB para considerar un archivo como "grande"
    #[serde(default = "default_large_threshold")]
    pub large_file_threshold_mib: u64,

    /// Usar planificador adaptativo para cargas mixtas
    #[serde(default = "default_true")]
    pub use_adaptive_scheduler: bool,

    /// Usar extensión temporal en el archivo de destino durante la copia
    #[serde(default = "default_true")]
    pub use_temporary_destination_extension: bool,

    /// Extensión temporal para archivos en proceso de copia
    #[serde(default = "default_temp_ext")]
    pub temporary_destination_extension: String,

    /// Modo append-only estricto: nunca sobrescribir destino
    #[serde(default = "default_true")]
    pub strict_append_only: bool,

    /// Eliminar archivo origen después de copia verificada (solo en modo move)
    #[serde(default)]
    pub delete_source_after_verified_copy: bool,

    /// Preservar jerarquía de directorios vacíos en destino
    #[serde(default = "default_true")]
    pub preserve_empty_directory_hierarchy: bool,

    /// Eliminar directorios vacíos del origen tras procesamiento exitoso
    #[serde(default = "default_true")]
    pub remove_empty_source_directories_after_successful_processing: bool,

    /// Ignorar enlaces simbólicos
    #[serde(default = "default_true")]
    pub ignore_symbolic_links: bool,

    /// Ignorar junctions de Windows
    #[serde(default = "default_true")]
    pub ignore_junctions: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationConfig {
    /// Modo de verificación: full_hash o none
    #[serde(default)]
    pub verification_mode: VerificationMode,

    /// Algoritmo de hash principal
    #[serde(default)]
    pub hash_algorithm: HashAlgorithm,

    /// Algoritmo de hash de respaldo
    #[serde(default = "default_fallback_hash")]
    pub fallback_hash_algorithm: HashAlgorithm,

    /// Solo marcar como exitoso si el hash coincide
    #[serde(default = "default_true")]
    pub delete_or_mark_success_only_after_hash_match: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetadataConfig {
    /// Preservar atributos del archivo (readonly, hidden, etc.) en el destino.
    /// Por defecto **false**: en NAS SMB sobre macOS/APFS, marcar solo lectura u otros
    /// flags NTFS suele provocar bloqueos en el servidor y fallos al renombrar `.partial`.
    #[serde(default = "default_false")]
    pub preserve_file_attributes: bool,

    /// Preservar fecha de creación
    #[serde(default = "default_true")]
    pub preserve_creation_time: bool,

    /// Preservar fecha de última escritura
    #[serde(default = "default_true")]
    pub preserve_last_write_time: bool,

    /// Preservar fecha de último acceso
    #[serde(default = "default_true")]
    pub preserve_last_access_time: bool,

    /// Preservar timestamps de directorios
    #[serde(default)]
    pub preserve_directory_timestamps: bool,

    /// Preservar ACL (no implementado en v1)
    #[serde(default)]
    pub preserve_acl: bool,

    /// Preservar propietario (no implementado en v1)
    #[serde(default)]
    pub preserve_owner: bool,

    /// Preservar auditoría (no implementado en v1)
    #[serde(default)]
    pub preserve_audit: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictConfig {
    /// Acción si el archivo de destino ya existe
    #[serde(default)]
    pub if_destination_file_exists: ConflictExistsAction,

    /// Acción si el hash es igual
    #[serde(default)]
    pub if_hash_is_equal: HashEqualAction,

    /// Acción si el hash es diferente
    #[serde(default)]
    pub if_hash_is_different: HashDifferentAction,

    /// Estrategia de naming para copias versionadas
    #[serde(default)]
    pub versioned_copy_naming_strategy: VersionedNamingStrategy,

    /// Nombre del directorio de conflictos
    #[serde(default = "default_conflict_dir")]
    pub conflict_directory_name: String,

    /// Registrar evento de conflicto en el log
    #[serde(default = "default_true")]
    pub write_conflict_event_to_log: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExclusionConfig {
    /// Nombres de directorio a excluir completamente
    #[serde(default = "default_excluded_dirs")]
    pub excluded_directory_names: Vec<String>,

    /// Patrones de archivo a excluir (glob)
    #[serde(default = "default_excluded_patterns")]
    pub excluded_file_patterns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicyConfig {
    /// Número de reintentos por archivo
    #[serde(default = "default_retries")]
    pub retries_per_file: u32,

    /// Secuencia de delays entre reintentos (segundos)
    #[serde(default = "default_retry_delays")]
    pub retry_delay_seconds_sequence: Vec<u64>,

    /// Delay de reintento cuando el NAS falla (segundos)
    #[serde(default = "default_nas_retry_delay")]
    pub nas_retry_delay_seconds: u64,

    /// Delay extendido ante error persistente (segundos)
    #[serde(default = "default_persistent_delay")]
    pub persistent_error_delay_seconds: u64,

    /// Continuar con otros archivos si uno falla
    #[serde(default = "default_true")]
    pub continue_on_file_error: bool,

    /// Número máximo de errores consecutivos antes de pausa extendida
    #[serde(default = "default_max_consecutive_errors")]
    pub maximum_consecutive_errors_before_extended_pause: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    /// Ruta del archivo de log humano (.log)
    pub human_log_file_path: PathBuf,

    /// Ruta del archivo CSV de métricas
    pub metrics_csv_file_path: PathBuf,

    /// Ruta del archivo JSON Lines de eventos
    pub events_jsonl_file_path: PathBuf,

    /// Tamaño máximo del archivo de log en MiB
    #[serde(default = "default_max_log_mib")]
    pub maximum_log_file_size_mib: u64,

    /// Número de archivos de log a mantener en rotación
    #[serde(default = "default_log_rotation")]
    pub log_rotation_keep_file_count: u32,

    /// Habilitar resumen diario
    #[serde(default = "default_true")]
    pub daily_summary_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledTaskConfig {
    /// Crear tarea programada
    #[serde(default = "default_true")]
    pub create_scheduled_task: bool,

    /// Nombre de la tarea programada
    #[serde(default = "default_task_name")]
    pub scheduled_task_name: String,

    /// Ejecutar solo cuando el usuario haya iniciado sesión
    #[serde(default = "default_true")]
    pub run_only_when_user_is_logged_on: bool,

    /// Ejecutar con privilegios elevados
    #[serde(default)]
    pub run_with_highest_privileges: bool,

    /// Iniciar al iniciar sesión del usuario
    #[serde(default = "default_true")]
    pub start_at_user_logon: bool,

    /// Retraso después del inicio de sesión (segundos)
    #[serde(default = "default_logon_delay")]
    pub delay_after_logon_seconds: u64,

    /// Impedir instancias paralelas
    #[serde(default = "default_true")]
    pub prevent_parallel_instances: bool,
}

// ─────────────────────────────────────────────
// Valores por defecto
// ─────────────────────────────────────────────

fn default_application_name() -> String {
    "Sincronia — Respaldo NAS".into()
}
fn default_language() -> String {
    "es-ES".into()
}
fn default_stable_seconds() -> u64 {
    60
}
fn default_scan_no_changes() -> u64 {
    15
}
fn default_scan_after_changes() -> u64 {
    10
}
fn default_true() -> bool {
    true
}
fn default_false() -> bool {
    false
}
fn default_worker_count() -> usize {
    8
}
fn default_max_workers() -> usize {
    16
}
fn default_buffer_mib() -> usize {
    16
}
fn default_large_threshold() -> u64 {
    1024
}
fn default_temp_ext() -> String {
    ".partial".into()
}
fn default_fallback_hash() -> HashAlgorithm {
    HashAlgorithm::Sha256
}
fn default_conflict_dir() -> String {
    "_conflicts".into()
}
fn default_excluded_dirs() -> Vec<String> {
    vec![
        ".stfolder".into(),
        ".stversions".into(),
        "$RECYCLE.BIN".into(),
        "System Volume Information".into(),
        ".Trashes".into(),
        ".Spotlight-V100".into(),
        ".fseventsd".into(),
    ]
}
fn default_excluded_patterns() -> Vec<String> {
    vec![
        "*.tmp".into(),
        "~*.*".into(),
        "Thumbs.db".into(),
        ".DS_Store".into(),
        "._*".into(),
        "desktop.ini".into(),
        "*.ffs_lock".into(),
        "*.ffs_db".into(),
        "*.partial".into(),
        "*.download".into(),
        "*.crdownload".into(),
        "*.lock".into(),
        "*.lck".into(),
    ]
}
fn default_retries() -> u32 {
    3
}
fn default_retry_delays() -> Vec<u64> {
    vec![2, 5, 15]
}
fn default_nas_retry_delay() -> u64 {
    60
}
fn default_persistent_delay() -> u64 {
    300
}
fn default_max_consecutive_errors() -> u32 {
    5
}
fn default_max_log_mib() -> u64 {
    50
}
fn default_log_rotation() -> u32 {
    10
}
fn default_task_name() -> String {
    "Sincronia NAS Backup".into()
}
fn default_logon_delay() -> u64 {
    30
}

// ─────────────────────────────────────────────
// Carga y validación
// ─────────────────────────────────────────────

fn is_valid_dos_drive_letter(s: &str) -> bool {
    let mut chars = s.chars();
    match (chars.next(), chars.next(), chars.next()) {
        (Some(c), Some(':'), None) => c.is_ascii_alphabetic(),
        _ => false,
    }
}

fn drive_letter_upper(letter: &str) -> Option<char> {
    let mut ch = letter.chars();
    match (ch.next(), ch.next(), ch.next()) {
        (Some(c), Some(':'), None) if c.is_ascii_alphabetic() => Some(c.to_ascii_uppercase()),
        _ => None,
    }
}

impl SincroniaConfig {
    /// Carga la configuración desde un archivo TOML
    pub fn load_from_file(path: &Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("No se pudo leer el archivo de configuración '{}': {}", path.display(), e))?;

        let config: SincroniaConfig = toml::from_str(&content).map_err(|e| {
            let err_str = e.to_string();
            let mut msg = format!(
                "Error al parsear el archivo TOML '{}': {}",
                path.display(),
                e
            );
            if err_str.contains("unicode")
                || err_str.contains("hex code")
                || err_str.contains("escape")
            {
                msg.push_str(
                    "\n\nSugerencia (rutas Windows en TOML): dentro de comillas dobles (\"...\"), \
                     la secuencia \\U inicia un carácter Unicode de 8 dígitos hexadecimales, \
                     por eso una ruta como \"C:\\Users\\...\" falla en \\Users. \
                     Use barras dobles: \"C:\\\\Users\\\\...\" o una cadena literal entre comillas simples: \
                     'C:\\Users\\...'",
                );
            }
            anyhow::anyhow!("{}", msg)
        })?;

        config.validate()?;
        Ok(config)
    }

    /// Valida la configuración cargada
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.sync_pairs.is_empty() {
            anyhow::bail!("sync_pairs no puede estar vacío: defina al menos un [[sync_pairs]]");
        }

        let mut used_letters = std::collections::HashSet::<char>::new();

        for (i, pair) in self.sync_pairs.iter().enumerate() {
            if !pair.source_path.exists() {
                anyhow::bail!(
                    "sync_pairs[{}]: el directorio origen no existe: {}",
                    i,
                    pair.source_path.display()
                );
            }

            if !pair.target_path.exists() {
                if let Some(parent) = pair.target_path.parent() {
                    if !parent.as_os_str().is_empty() && !parent.exists() {
                        anyhow::bail!(
                            "sync_pairs[{}]: el destino no existe y el directorio padre tampoco: {}",
                            i,
                            pair.target_path.display()
                        );
                    }
                }
            }

            for (label, letter) in [
                ("source_virtual_drive_letter", pair.source_virtual_drive_letter.as_str()),
                ("target_virtual_drive_letter", pair.target_virtual_drive_letter.as_str()),
            ] {
                if !is_valid_dos_drive_letter(letter) {
                    anyhow::bail!(
                        "sync_pairs[{}]: {} inválida '{}'. Formato esperado: 'X:'",
                        i,
                        label,
                        letter
                    );
                }
            }

            let src_u = drive_letter_upper(&pair.source_virtual_drive_letter)
                .ok_or_else(|| anyhow::anyhow!("sync_pairs[{}]: letra de origen inválida", i))?;
            let tgt_u = drive_letter_upper(&pair.target_virtual_drive_letter)
                .ok_or_else(|| anyhow::anyhow!("sync_pairs[{}]: letra de destino inválida", i))?;

            if src_u == tgt_u {
                anyhow::bail!(
                    "sync_pairs[{}]: la letra virtual de origen y destino deben ser distintas ('{}')",
                    i,
                    pair.source_virtual_drive_letter
                );
            }

            for (which, c) in [("origen", src_u), ("destino", tgt_u)] {
                if !used_letters.insert(c) {
                    anyhow::bail!(
                        "sync_pairs[{}]: la letra '{}' ({}) ya está en uso en otro par",
                        i,
                        c,
                        which
                    );
                }
            }
        }

        // Validar worker count
        if self.copy_engine.worker_count == 0 {
            anyhow::bail!("worker_count debe ser al menos 1");
        }
        if self.copy_engine.worker_count > self.copy_engine.maximum_worker_count {
            anyhow::bail!(
                "worker_count ({}) no puede ser mayor que maximum_worker_count ({})",
                self.copy_engine.worker_count,
                self.copy_engine.maximum_worker_count
            );
        }

        // Validar buffer size
        if self.copy_engine.copy_buffer_size_mib_per_worker == 0 {
            anyhow::bail!("copy_buffer_size_mib_per_worker debe ser al menos 1 MiB");
        }

        // Validar directorio de logs (crear si no existe)
        if let Some(parent) = self.logging.human_log_file_path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent)?;
            }
        }
        if let Some(parent) = self.logging.metrics_csv_file_path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent)?;
            }
        }
        if let Some(parent) = self.logging.events_jsonl_file_path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent)?;
            }
        }

        // Validar retry delays
        if self.retry_policy.retry_delay_seconds_sequence.is_empty() {
            anyhow::bail!("retry_delay_seconds_sequence no puede estar vacía");
        }

        Ok(())
    }

    /// Genera el contenido TOML de ejemplo con todos los parámetros documentados
    pub fn generate_example_toml() -> String {
        include_str!("../sincronia.example.toml").to_string()
    }
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_valid_toml() {
        let toml_str = r#"
[general]
application_name = "Test App"
run_mode = "backup_append_only"
language = "es-ES"

[scan]
scan_interval_seconds_when_no_changes = 15
scan_interval_seconds_after_changes = 10

[[sync_pairs]]
source_path = "."
source_virtual_drive_letter = "X:"
target_path = "."
target_virtual_drive_letter = "Y:"
minimum_file_stable_seconds = 30

[copy_engine]
worker_count = 4

[verification]
verification_mode = "full_hash"
hash_algorithm = "blake3"

[metadata]

[conflicts]
if_destination_file_exists = "hash_compare"

[exclusions]
excluded_directory_names = [".stfolder"]
excluded_file_patterns = ["*.tmp"]

[retry_policy]
retries_per_file = 3
retry_delay_seconds_sequence = [2, 5, 15]

[logging]
human_log_file_path = "./test.log"
metrics_csv_file_path = "./test.csv"
events_jsonl_file_path = "./test.jsonl"

[scheduled_task]
create_scheduled_task = false
"#;
        let config: SincroniaConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.general.application_name, "Test App");
        assert_eq!(config.copy_engine.worker_count, 4);
        assert_eq!(config.sync_pairs.len(), 1);
        assert_eq!(config.sync_pairs[0].source_virtual_drive_letter, "X:");
        assert_eq!(config.sync_pairs[0].target_virtual_drive_letter, "Y:");
        assert!(!config.metadata.preserve_file_attributes);
    }

    #[test]
    fn test_invalid_drive_letter() {
        let toml_str = r#"
[general]
application_name = "Test"
run_mode = "backup_append_only"
language = "es-ES"

[scan]

[[sync_pairs]]
source_path = "."
source_virtual_drive_letter = "ZZ"
target_path = "."
target_virtual_drive_letter = "Y:"

[copy_engine]

[verification]

[metadata]

[conflicts]

[exclusions]

[retry_policy]
retry_delay_seconds_sequence = [2]

[logging]
human_log_file_path = "./test.log"
metrics_csv_file_path = "./test.csv"
events_jsonl_file_path = "./test.jsonl"

[scheduled_task]
"#;
        let config: SincroniaConfig = toml::from_str(toml_str).unwrap();
        let result = config.validate();
        assert!(result.is_err());
    }

    #[test]
    fn test_run_mode_default() {
        assert_eq!(RunMode::default(), RunMode::BackupAppendOnly);
    }
}
