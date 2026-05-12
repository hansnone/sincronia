// sincronia/src/verifier.rs
//
// Verificación post-copia: calcula hash del origen y del temporal de destino,
// compara para garantizar integridad.

use crate::config::VerificationConfig;
use crate::errors::{HashAlgorithm, SincroniaError, VerificationMode};
use crate::hasher;
use std::path::Path;
use std::time::Instant;
use tracing::{debug, info, warn};

/// Resultado de la verificación
#[derive(Debug, Clone)]
pub struct VerificationResult {
    /// Si la verificación fue exitosa
    pub success: bool,
    /// Hash del archivo origen
    pub source_hash: String,
    /// Hash del archivo destino
    pub destination_hash: String,
    /// Algoritmo utilizado
    pub algorithm: HashAlgorithm,
    /// Duración del hash en milisegundos
    pub hash_duration_ms: u64,
}

/// Verifica la integridad de un archivo copiado comparando hashes
pub fn verify_copy(
    source_path: &Path,
    destination_path: &Path,
    config: &VerificationConfig,
) -> Result<VerificationResult, SincroniaError> {
    match config.verification_mode {
        VerificationMode::None => {
            debug!("Verificación deshabilitada — saltando");
            Ok(VerificationResult {
                success: true,
                source_hash: String::new(),
                destination_hash: String::new(),
                algorithm: config.hash_algorithm.clone(),
                hash_duration_ms: 0,
            })
        }
        VerificationMode::FullHash => {
            verify_full_hash(source_path, destination_path, &config.hash_algorithm, &config.fallback_hash_algorithm)
        }
    }
}

/// Verificación completa por hash
fn verify_full_hash(
    source_path: &Path,
    destination_path: &Path,
    primary_algorithm: &HashAlgorithm,
    fallback_algorithm: &HashAlgorithm,
) -> Result<VerificationResult, SincroniaError> {
    let start = Instant::now();

    info!(
        "Verificando integridad ({:?}): {} vs {}",
        primary_algorithm,
        source_path.display(),
        destination_path.display()
    );

    // Intentar con algoritmo principal
    let result = verify_with_algorithm(source_path, destination_path, primary_algorithm);

    match result {
        Ok(r) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            Ok(VerificationResult {
                hash_duration_ms: duration_ms,
                ..r
            })
        }
        Err(e) => {
            // Si falla el principal, intentar con fallback
            warn!(
                "Hash principal ({:?}) falló: {}. Intentando con {:?}",
                primary_algorithm, e, fallback_algorithm
            );
            let result = verify_with_algorithm(source_path, destination_path, fallback_algorithm)?;
            let duration_ms = start.elapsed().as_millis() as u64;
            Ok(VerificationResult {
                hash_duration_ms: duration_ms,
                ..result
            })
        }
    }
}

fn verify_with_algorithm(
    source_path: &Path,
    destination_path: &Path,
    algorithm: &HashAlgorithm,
) -> Result<VerificationResult, SincroniaError> {
    let source_hash = hasher::hash_file(source_path, algorithm)?;
    let dest_hash = hasher::hash_file(destination_path, algorithm)?;

    let success = source_hash.hex == dest_hash.hex;

    if success {
        debug!(
            "Verificación exitosa: {} ({:?}: {}...)",
            source_path.display(),
            algorithm,
            &source_hash.hex[..std::cmp::min(16, source_hash.hex.len())]
        );
    } else {
        warn!(
            "¡VERIFICACIÓN FALLIDA! {} — origen: {}..., destino: {}...",
            source_path.display(),
            &source_hash.hex[..std::cmp::min(16, source_hash.hex.len())],
            &dest_hash.hex[..std::cmp::min(16, dest_hash.hex.len())]
        );
    }

    Ok(VerificationResult {
        success,
        source_hash: source_hash.hex,
        destination_hash: dest_hash.hex,
        algorithm: algorithm.clone(),
        hash_duration_ms: 0, // Se sobreescribe en verify_copy
    })
}
