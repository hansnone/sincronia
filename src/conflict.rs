// sincronia/src/conflict.rs
//
// Gestión de conflictos cuando el archivo de destino ya existe.
// Soporta comparación por hash, copias versionadas con timestamp,
// y directorio de conflictos.

use crate::config::ConflictConfig;
use crate::errors::{
    ConflictExistsAction, FileState, HashAlgorithm, HashDifferentAction, HashEqualAction,
    SincroniaError, VersionedNamingStrategy,
};
use crate::hasher;
use chrono::Local;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Resultado de la resolución de conflicto
#[derive(Debug, Clone)]
pub enum ConflictResolution {
    /// No hay conflicto — destino no existe
    NoConflict,
    /// Archivo idéntico ya existe
    AlreadyExists,
    /// Se debe crear copia versionada en esta ruta
    CreateVersionedCopy(PathBuf),
    /// Error de conflicto irrecuperable
    Error(String),
}

/// Resuelve un conflicto cuando el archivo de destino ya existe
pub fn resolve_conflict(
    source_path: &Path,
    destination_path: &Path,
    config: &ConflictConfig,
    hash_algorithm: &HashAlgorithm,
) -> Result<(ConflictResolution, FileState), SincroniaError> {
    // Si el destino no existe, no hay conflicto
    if !destination_path.exists() {
        return Ok((ConflictResolution::NoConflict, FileState::Pending));
    }

    debug!(
        "Conflicto detectado: destino ya existe: {}",
        destination_path.display()
    );

    match &config.if_destination_file_exists {
        ConflictExistsAction::Skip => {
            info!("Conflicto resuelto: saltando {}", destination_path.display());
            Ok((
                ConflictResolution::AlreadyExists,
                FileState::AlreadyExistsSameHash,
            ))
        }
        ConflictExistsAction::AlwaysVersion => {
            let versioned = generate_versioned_path(destination_path, &config.versioned_copy_naming_strategy);
            info!(
                "Conflicto resuelto: creando versión {}",
                versioned.display()
            );
            Ok((
                ConflictResolution::CreateVersionedCopy(versioned),
                FileState::VersionedCopyCreated,
            ))
        }
        ConflictExistsAction::HashCompare => {
            // Comparar hashes
            info!(
                "Comparando hash de origen y destino existente ({})",
                destination_path.display()
            );

            let source_hash = hasher::hash_file(source_path, hash_algorithm)?;
            let dest_hash = hasher::hash_file(destination_path, hash_algorithm)?;

            if source_hash.hex == dest_hash.hex {
                // Hash igual
                match &config.if_hash_is_equal {
                    HashEqualAction::MarkAsAlreadyBackedUp => {
                        info!(
                            "Hash idéntico — marcado como ya respaldado: {}",
                            destination_path.display()
                        );
                        Ok((
                            ConflictResolution::AlreadyExists,
                            FileState::AlreadyExistsSameHash,
                        ))
                    }
                    HashEqualAction::Skip => {
                        debug!(
                            "Hash idéntico — saltando: {}",
                            destination_path.display()
                        );
                        Ok((
                            ConflictResolution::AlreadyExists,
                            FileState::AlreadyExistsSameHash,
                        ))
                    }
                }
            } else {
                // Hash diferente
                warn!(
                    "Hash DIFERENTE para {}: origen={}, destino={}",
                    destination_path.display(),
                    &source_hash.hex[..16],
                    &dest_hash.hex[..16]
                );

                match &config.if_hash_is_different {
                    HashDifferentAction::CreateVersionedCopy => {
                        let versioned = generate_versioned_path(
                            destination_path,
                            &config.versioned_copy_naming_strategy,
                        );
                        info!(
                            "Hash diferente — creando copia versionada: {}",
                            versioned.display()
                        );
                        Ok((
                            ConflictResolution::CreateVersionedCopy(versioned),
                            FileState::VersionedCopyCreated,
                        ))
                    }
                    HashDifferentAction::MoveToConflictDirectory => {
                        let conflict_dir = destination_path
                            .parent()
                            .unwrap_or(Path::new("."))
                            .join(&config.conflict_directory_name);
                        let conflict_path = conflict_dir.join(
                            destination_path
                                .file_name()
                                .unwrap_or_default(),
                        );
                        let versioned = generate_versioned_path(
                            &conflict_path,
                            &config.versioned_copy_naming_strategy,
                        );
                        info!(
                            "Hash diferente — enviando a directorio de conflictos: {}",
                            versioned.display()
                        );
                        Ok((
                            ConflictResolution::CreateVersionedCopy(versioned),
                            FileState::ConflictDestinationExistsDifferentHash,
                        ))
                    }
                }
            }
        }
        ConflictExistsAction::SizeAndMtime => {
            info!(
                "Comparando tamaño y fecha de modificación para {}",
                destination_path.display()
            );

            let source_meta = std::fs::metadata(source_path).map_err(|e| SincroniaError::Io {
                path: source_path.to_path_buf(),
                source: e,
            })?;
            let dest_meta = std::fs::metadata(destination_path).map_err(|e| SincroniaError::Io {
                path: destination_path.to_path_buf(),
                source: e,
            })?;

            let source_size = source_meta.len();
            let dest_size = dest_meta.len();

            let source_mtime = source_meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            let dest_mtime = dest_meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH);

            // Toleramos una diferencia de hasta 2 segundos para compensar la precisión de SMB/FAT
            let mtime_diff = source_mtime.duration_since(dest_mtime)
                .unwrap_or_else(|e| e.duration())
                .as_secs();

            if source_size == dest_size && mtime_diff <= 2 {
                match &config.if_hash_is_equal {
                    HashEqualAction::MarkAsAlreadyBackedUp => {
                        info!(
                            "Tamaño y MTime iguales — marcado como ya respaldado: {}",
                            destination_path.display()
                        );
                        Ok((
                            ConflictResolution::AlreadyExists,
                            FileState::AlreadyExistsSameHash,
                        ))
                    }
                    HashEqualAction::Skip => {
                        debug!(
                            "Tamaño y MTime iguales — saltando: {}",
                            destination_path.display()
                        );
                        Ok((
                            ConflictResolution::AlreadyExists,
                            FileState::AlreadyExistsSameHash,
                        ))
                    }
                }
            } else {
                warn!(
                    "Tamaño o MTime DIFERENTE para {}: size({} vs {}), mtime_diff={}s",
                    destination_path.display(),
                    source_size, dest_size, mtime_diff
                );

                match &config.if_hash_is_different {
                    HashDifferentAction::CreateVersionedCopy => {
                        let versioned = generate_versioned_path(
                            destination_path,
                            &config.versioned_copy_naming_strategy,
                        );
                        info!(
                            "Tamaño/MTime diferente — creando copia versionada: {}",
                            versioned.display()
                        );
                        Ok((
                            ConflictResolution::CreateVersionedCopy(versioned),
                            FileState::VersionedCopyCreated,
                        ))
                    }
                    HashDifferentAction::MoveToConflictDirectory => {
                        let conflict_dir = destination_path
                            .parent()
                            .unwrap_or(Path::new("."))
                            .join(&config.conflict_directory_name);
                        let conflict_path = conflict_dir.join(
                            destination_path
                                .file_name()
                                .unwrap_or_default(),
                        );
                        let versioned = generate_versioned_path(
                            &conflict_path,
                            &config.versioned_copy_naming_strategy,
                        );
                        info!(
                            "Tamaño/MTime diferente — enviando a directorio de conflictos: {}",
                            versioned.display()
                        );
                        Ok((
                            ConflictResolution::CreateVersionedCopy(versioned),
                            FileState::ConflictDestinationExistsDifferentHash,
                        ))
                    }
                }
            }
        }
    }
}

/// Genera una ruta versionada añadiendo sufijo al nombre de archivo
pub fn generate_versioned_path(
    original_path: &Path,
    strategy: &VersionedNamingStrategy,
) -> PathBuf {
    let parent = original_path.parent().unwrap_or(Path::new("."));
    let stem = original_path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy();
    let extension = original_path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();

    match strategy {
        VersionedNamingStrategy::TimestampSuffix => {
            let timestamp = Local::now().format("%Y%m%d_%H%M%S");
            let new_name = format!("{}_{}{}", stem, timestamp, extension);
            parent.join(new_name)
        }
        VersionedNamingStrategy::NumericSuffix => {
            // Buscar el siguiente número disponible
            for i in 1..=9999 {
                let new_name = format!("{}_{:03}{}", stem, i, extension);
                let candidate = parent.join(&new_name);
                if !candidate.exists() {
                    return candidate;
                }
            }
            // Fallback con timestamp si se agotan los números
            let timestamp = Local::now().format("%Y%m%d_%H%M%S");
            parent.join(format!("{}_{}{}", stem, timestamp, extension))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_versioned_path_timestamp() {
        let path = Path::new("C:\\backup\\video.mp4");
        let versioned =
            generate_versioned_path(path, &VersionedNamingStrategy::TimestampSuffix);

        let name = versioned.file_name().unwrap().to_string_lossy();
        assert!(name.starts_with("video_"));
        assert!(name.ends_with(".mp4"));
        assert!(name.len() > "video_.mp4".len());
    }

    #[test]
    fn test_versioned_path_no_extension() {
        let path = Path::new("C:\\backup\\README");
        let versioned =
            generate_versioned_path(path, &VersionedNamingStrategy::TimestampSuffix);

        let name = versioned.file_name().unwrap().to_string_lossy();
        assert!(name.starts_with("README_"));
        assert!(!name.contains("."));
    }

    #[test]
    fn test_versioned_path_numeric() {
        let path = Path::new("C:\\backup\\photo.jpg");
        let versioned =
            generate_versioned_path(path, &VersionedNamingStrategy::NumericSuffix);

        let name = versioned.file_name().unwrap().to_string_lossy();
        // Como C:\backup\photo_001.jpg no existe, debería ser _001
        assert!(name.starts_with("photo_"));
        assert!(name.ends_with(".jpg"));
    }
}
