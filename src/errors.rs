// sincronia/src/errors.rs
//
// Tipos de error centralizados y enums de estado para todo el sistema.
// Usa `thiserror` para derivar Display/Error automáticamente.

use std::path::PathBuf;
use thiserror::Error;

// ─────────────────────────────────────────────
// Error principal del sistema
// ─────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum SincroniaError {
    #[error("Error de configuración: {message}")]
    Config { message: String },

    #[error("Error de E/S en '{path}': {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("Error de NAS: {message}")]
    Nas { message: String },

    #[error("Error de credenciales: {message}")]
    Credential { message: String },

    #[error("Error de copia en '{path}': {message}")]
    Copy { path: PathBuf, message: String },

    #[error("Error de hash en '{path}': {message}")]
    Hash { path: PathBuf, message: String },

    #[error("Error de metadatos en '{path}': {message}")]
    Metadata { path: PathBuf, message: String },

    #[error("Error de permisos en '{path}': {message}")]
    Permission { path: PathBuf, message: String },

    #[error("Conflicto en destino '{path}': {message}")]
    Conflict { path: PathBuf, message: String },

    #[error("Error de Windows API: {message} (código: {code})")]
    WindowsApi { message: String, code: u32 },

    #[error("Operación cancelada por el usuario")]
    Cancelled,

    #[error("Parada ordenada en curso")]
    ShuttingDown,
}

// ─────────────────────────────────────────────
// Estado por archivo durante el procesamiento
// ─────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileState {
    /// Archivo detectado por el scanner
    Detected,
    /// Excluido por patrón de exclusión
    Excluded,
    /// Inestable: tamaño o mtime cambiando
    Unstable,
    /// Pendiente de copia (estable y listo)
    Pending,
    /// En proceso de copia
    Copying,
    /// Copiado al archivo temporal en destino
    CopiedToTemporary,
    /// En proceso de verificación de hash
    Verifying,
    /// Hash verificado correctamente
    Verified,
    /// Metadatos aplicados correctamente
    MetadataApplied,
    /// Archivo finalizado (renombrado a nombre final)
    Finalized,
    /// Ya existía en destino con el mismo hash
    AlreadyExistsSameHash,
    /// Se creó copia versionada (destino existía con hash diferente)
    VersionedCopyCreated,
    /// Conflicto: destino existe con hash diferente
    ConflictDestinationExistsDifferentHash,
    /// Error de lectura del origen
    FailedRead,
    /// Error de escritura en destino
    FailedWrite,
    /// Error de hash
    FailedHash,
    /// Error al aplicar metadatos
    FailedMetadata,
    /// Error al renombrar archivo final
    FailedFinalize,
    /// Saltado tras agotar reintentos
    SkippedAfterRetries,
}

impl std::fmt::Display for FileState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            Self::Detected => "detectado",
            Self::Excluded => "excluido",
            Self::Unstable => "inestable",
            Self::Pending => "pendiente",
            Self::Copying => "copiando",
            Self::CopiedToTemporary => "copiado_a_temporal",
            Self::Verifying => "verificando",
            Self::Verified => "verificado",
            Self::MetadataApplied => "metadatos_aplicados",
            Self::Finalized => "finalizado",
            Self::AlreadyExistsSameHash => "ya_existe_mismo_hash",
            Self::VersionedCopyCreated => "copia_versionada_creada",
            Self::ConflictDestinationExistsDifferentHash => "conflicto_hash_diferente",
            Self::FailedRead => "error_lectura",
            Self::FailedWrite => "error_escritura",
            Self::FailedHash => "error_hash",
            Self::FailedMetadata => "error_metadatos",
            Self::FailedFinalize => "error_finalización",
            Self::SkippedAfterRetries => "saltado_tras_reintentos",
        };
        write!(f, "{}", label)
    }
}

impl FileState {
    /// Devuelve true si el estado representa un error
    pub fn is_error(&self) -> bool {
        matches!(
            self,
            Self::FailedRead
                | Self::FailedWrite
                | Self::FailedHash
                | Self::FailedMetadata
                | Self::FailedFinalize
                | Self::SkippedAfterRetries
        )
    }

    /// Devuelve true si el estado representa éxito final
    pub fn is_success(&self) -> bool {
        matches!(
            self,
            Self::Finalized | Self::AlreadyExistsSameHash | Self::VersionedCopyCreated
        )
    }
}

// ─────────────────────────────────────────────
// Estado global del sistema
// ─────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GlobalState {
    /// Iniciando el sistema
    Starting,
    /// Cargando configuración
    LoadingConfiguration,
    /// Validando configuración
    ValidatingConfiguration,
    /// Escaneando directorio origen
    Scanning,
    /// Copiando archivos (detalle del par actual)
    Copying(String),
    /// Inactivo, esperando cambios
    Idle,
    /// Pausado por el usuario
    Paused,
    /// Error transitorio (reintentando)
    ErrorTransient,
    /// Error persistente (espera extendida)
    ErrorPersistent,
    /// Parando de forma ordenada
    Stopping,
    /// Parado completamente
    Stopped,
}

impl GlobalState {
    /// Color del icono de bandeja para este estado
    pub fn tray_color(&self) -> TrayColor {
        match self {
            Self::Scanning | Self::Copying(_) | Self::Idle => TrayColor::Green,
            Self::Starting
            | Self::LoadingConfiguration
            | Self::ValidatingConfiguration
            | Self::Paused => TrayColor::Yellow,
            Self::ErrorTransient
            | Self::ErrorPersistent
            | Self::Stopping
            | Self::Stopped => TrayColor::Red,
        }
    }
}

impl std::fmt::Display for GlobalState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            Self::Starting => "Iniciando",
            Self::LoadingConfiguration => "Cargando configuración",
            Self::ValidatingConfiguration => "Validando configuración",
            Self::Scanning => "Escaneando",
            Self::Copying(detail) => {
                return write!(f, "Copiando: {}", detail);
            }
            Self::Idle => "Inactivo",
            Self::Paused => "Pausado",
            Self::ErrorTransient => "Error transitorio",
            Self::ErrorPersistent => "Error persistente",
            Self::Stopping => "Parando",
            Self::Stopped => "Parado",
        };
        write!(f, "{}", label)
    }
}

// ─────────────────────────────────────────────
// Color del icono de bandeja
// ─────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayColor {
    /// NAS disponible y motor funcionando
    Green,
    /// NAS no montado, esperando, pausado
    Yellow,
    /// Error persistente, NAS inaccesible, fallo grave
    Red,
}

// ─────────────────────────────────────────────
// Modo de ejecución
// ─────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunMode {
    /// Respaldar archivos, conservar origen, crear versiones si hay cambios
    BackupAppendOnly,
    /// Copiar, verificar hash completo, eliminar origen tras verificación
    MoveAfterVerifiedCopy,
}

impl Default for RunMode {
    fn default() -> Self {
        Self::BackupAppendOnly
    }
}

// ─────────────────────────────────────────────
// Modo de verificación
// ─────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationMode {
    /// Hash completo del archivo
    FullHash,
    /// Sin verificación (no recomendado)
    None,
}

impl Default for VerificationMode {
    fn default() -> Self {
        Self::FullHash
    }
}

// ─────────────────────────────────────────────
// Algoritmo de hash
// ─────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HashAlgorithm {
    Blake3,
    Sha256,
}

impl Default for HashAlgorithm {
    fn default() -> Self {
        Self::Blake3
    }
}

// ─────────────────────────────────────────────
// Estrategia de conflicto
// ─────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictExistsAction {
    /// Comparar hash para decidir
    HashCompare,
    /// Saltar el archivo
    Skip,
    /// Crear siempre copia versionada
    AlwaysVersion,
}

impl Default for ConflictExistsAction {
    fn default() -> Self {
        Self::HashCompare
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HashEqualAction {
    /// Marcar como ya respaldado
    MarkAsAlreadyBackedUp,
    /// Saltar silenciosamente
    Skip,
}

impl Default for HashEqualAction {
    fn default() -> Self {
        Self::MarkAsAlreadyBackedUp
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HashDifferentAction {
    /// Crear copia con sufijo de timestamp
    CreateVersionedCopy,
    /// Mover a directorio de conflictos
    MoveToConflictDirectory,
}

impl Default for HashDifferentAction {
    fn default() -> Self {
        Self::CreateVersionedCopy
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VersionedNamingStrategy {
    /// Sufijo con timestamp: archivo_20260512_220300.ext
    TimestampSuffix,
    /// Sufijo numérico incremental: archivo_001.ext
    NumericSuffix,
}

impl Default for VersionedNamingStrategy {
    fn default() -> Self {
        Self::TimestampSuffix
    }
}
