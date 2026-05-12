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
    pub source: SourceConfig,
    pub nas: NasConfig,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceConfig {
    /// Ruta del directorio origen a monitorizar
    pub source_directory_path: PathBuf,

    /// Segundos que un archivo debe permanecer sin cambios para considerarse estable
    #[serde(default = "default_stable_seconds")]
    pub minimum_file_stable_seconds: u64,

    /// Intervalo de escaneo en segundos cuando no hay cambios
    #[serde(default = "default_scan_no_changes")]
    pub scan_interval_seconds_when_no_changes: u64,

    /// Intervalo de escaneo en segundos después de detectar cambios
    #[serde(default = "default_scan_after_changes")]
    pub scan_interval_seconds_after_changes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NasConfig {
    /// Letra de unidad obligatoria (ej: "R:")
    pub required_drive_letter: String,

    /// Ruta UNC primaria del NAS (ej: "\\\\RAW-NAS\\Repositorio")
    pub primary_unc_path: String,

    /// Ruta UNC de fallback por IP (ej: "\\\\10.71.11.41\\Repositorio")
    #[serde(default)]
    pub fallback_unc_path_by_ip: String,

    /// Permitir fallback por IP (deshabilitado por defecto para priorizar Kerberos)
    #[serde(default)]
    pub allow_ip_fallback: bool,

    /// Permitir remapeo automático si R: apunta a otro recurso
    #[serde(default)]
    pub allow_automatic_remap_if_drive_points_elsewhere: bool,

    /// Preferir API WNet de Windows como vía principal
    #[serde(default = "default_true")]
    pub prefer_windows_wnet_api: bool,

    /// Permitir fallback mediante net use si WNet falla
    #[serde(default = "default_true")]
    pub allow_net_use_fallback: bool,

    /// Número máximo de intentos para solicitar credenciales
    #[serde(default = "default_max_cred_attempts")]
    pub maximum_credential_prompt_attempts: u32,
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
    /// Preservar atributos del archivo (readonly, hidden, etc.)
    #[serde(default = "default_true")]
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
fn default_max_cred_attempts() -> u32 {
    3
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

impl SincroniaConfig {
    /// Carga la configuración desde un archivo TOML
    pub fn load_from_file(path: &Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("No se pudo leer el archivo de configuración '{}': {}", path.display(), e))?;

        let config: SincroniaConfig = toml::from_str(&content)
            .map_err(|e| anyhow::anyhow!("Error al parsear el archivo TOML '{}': {}", path.display(), e))?;

        config.validate()?;
        Ok(config)
    }

    /// Valida la configuración cargada
    pub fn validate(&self) -> anyhow::Result<()> {
        // Validar que el directorio origen existe
        if !self.source.source_directory_path.exists() {
            anyhow::bail!(
                "El directorio origen no existe: {}",
                self.source.source_directory_path.display()
            );
        }

        // Validar letra de unidad
        let letter = &self.nas.required_drive_letter;
        if letter.len() != 2 || !letter.ends_with(':') || !letter.chars().next().unwrap_or(' ').is_ascii_alphabetic() {
            anyhow::bail!(
                "Letra de unidad inválida: '{}'. Formato esperado: 'R:'",
                letter
            );
        }

        // Validar UNC path
        if !self.nas.primary_unc_path.starts_with("\\\\") {
            anyhow::bail!(
                "Ruta UNC primaria inválida: '{}'. Debe comenzar con \\\\",
                self.nas.primary_unc_path
            );
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

[source]
source_directory_path = "."
minimum_file_stable_seconds = 30

[nas]
required_drive_letter = "R:"
primary_unc_path = "\\\\RAW-NAS\\Repositorio"

[copy_engine]
worker_count = 4

[verification]
verification_mode = "full_hash"
hash_algorithm = "blake3"

[metadata]
preserve_file_attributes = true

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
        assert_eq!(config.nas.required_drive_letter, "R:");
    }

    #[test]
    fn test_invalid_drive_letter() {
        let toml_str = r#"
[general]
application_name = "Test"
run_mode = "backup_append_only"
language = "es-ES"

[source]
source_directory_path = "."

[nas]
required_drive_letter = "XYZ"
primary_unc_path = "\\\\RAW-NAS\\Repositorio"

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
