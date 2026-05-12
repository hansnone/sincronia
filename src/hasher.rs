// sincronia/src/hasher.rs
//
// Hashing de archivos con BLAKE3 (principal) y SHA-256 (alternativa).
// Lectura en streaming con buffers grandes para rendimiento óptimo.
// BLAKE3 usa SIMD automáticamente (AVX-512 en i9 modernos, ~5 GB/s).

use crate::errors::{HashAlgorithm, SincroniaError};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use tracing::{debug, trace};

/// Tamaño de buffer para hashing (4 MiB — equilibrio entre RAM y syscalls)
const HASH_BUFFER_SIZE: usize = 4 * 1024 * 1024;

/// Resultado de hash con metadatos
#[derive(Debug, Clone)]
pub struct HashResult {
    /// Hash en formato hexadecimal
    pub hex: String,
    /// Algoritmo utilizado
    pub algorithm: HashAlgorithm,
    /// Bytes procesados
    pub bytes_processed: u64,
}

/// Calcula el hash de un archivo usando el algoritmo especificado
pub fn hash_file(path: &Path, algorithm: &HashAlgorithm) -> Result<HashResult, SincroniaError> {
    debug!(
        "Calculando hash {:?} de: {}",
        algorithm,
        path.display()
    );

    match algorithm {
        HashAlgorithm::Blake3 => hash_file_blake3(path),
        HashAlgorithm::Sha256 => hash_file_sha256(path),
    }
}

/// Hash con BLAKE3 (streaming, SIMD-accelerated)
fn hash_file_blake3(path: &Path) -> Result<HashResult, SincroniaError> {
    let mut file = File::open(path).map_err(|e| SincroniaError::Hash {
        path: path.to_path_buf(),
        message: format!("No se pudo abrir para hash: {}", e),
    })?;

    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0u8; HASH_BUFFER_SIZE];
    let mut total_bytes: u64 = 0;

    loop {
        let bytes_read = file.read(&mut buffer).map_err(|e| SincroniaError::Hash {
            path: path.to_path_buf(),
            message: format!("Error de lectura durante hash: {}", e),
        })?;

        if bytes_read == 0 {
            break;
        }

        hasher.update(&buffer[..bytes_read]);
        total_bytes += bytes_read as u64;
    }

    let hash = hasher.finalize();
    let hex = hash.to_hex().to_string();

    trace!(
        "BLAKE3 de {} ({} bytes): {}",
        path.display(),
        total_bytes,
        &hex[..16]
    );

    Ok(HashResult {
        hex,
        algorithm: HashAlgorithm::Blake3,
        bytes_processed: total_bytes,
    })
}

/// Hash con SHA-256 (streaming)
fn hash_file_sha256(path: &Path) -> Result<HashResult, SincroniaError> {
    let mut file = File::open(path).map_err(|e| SincroniaError::Hash {
        path: path.to_path_buf(),
        message: format!("No se pudo abrir para hash: {}", e),
    })?;

    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; HASH_BUFFER_SIZE];
    let mut total_bytes: u64 = 0;

    loop {
        let bytes_read = file.read(&mut buffer).map_err(|e| SincroniaError::Hash {
            path: path.to_path_buf(),
            message: format!("Error de lectura durante hash: {}", e),
        })?;

        if bytes_read == 0 {
            break;
        }

        hasher.update(&buffer[..bytes_read]);
        total_bytes += bytes_read as u64;
    }

    let hash = hasher.finalize();
    let hex = format!("{:x}", hash);

    trace!(
        "SHA-256 de {} ({} bytes): {}",
        path.display(),
        total_bytes,
        &hex[..16]
    );

    Ok(HashResult {
        hex,
        algorithm: HashAlgorithm::Sha256,
        bytes_processed: total_bytes,
    })
}

/// Compara los hashes de dos archivos
pub fn compare_files(
    path_a: &Path,
    path_b: &Path,
    algorithm: &HashAlgorithm,
) -> Result<bool, SincroniaError> {
    let hash_a = hash_file(path_a, algorithm)?;
    let hash_b = hash_file(path_b, algorithm)?;

    let equal = hash_a.hex == hash_b.hex;
    debug!(
        "Comparación {:?}: {} vs {} → {}",
        algorithm,
        path_a.display(),
        path_b.display(),
        if equal { "IGUALES" } else { "DIFERENTES" }
    );

    Ok(equal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_blake3_hash_consistency() {
        let dir = std::env::temp_dir().join("sincronia_test_hash");
        std::fs::create_dir_all(&dir).unwrap();
        let file_path = dir.join("test_blake3.bin");

        // Escribir contenido conocido
        let mut file = File::create(&file_path).unwrap();
        file.write_all(b"Hello, Sincronia!").unwrap();
        drop(file);

        // Hash dos veces — debe dar el mismo resultado
        let hash1 = hash_file(&file_path, &HashAlgorithm::Blake3).unwrap();
        let hash2 = hash_file(&file_path, &HashAlgorithm::Blake3).unwrap();
        assert_eq!(hash1.hex, hash2.hex);
        assert_eq!(hash1.bytes_processed, 17);

        // Limpiar
        std::fs::remove_file(&file_path).ok();
        std::fs::remove_dir(&dir).ok();
    }

    #[test]
    fn test_sha256_hash_consistency() {
        let dir = std::env::temp_dir().join("sincronia_test_hash_sha");
        std::fs::create_dir_all(&dir).unwrap();
        let file_path = dir.join("test_sha256.bin");

        let mut file = File::create(&file_path).unwrap();
        file.write_all(b"Hello, Sincronia!").unwrap();
        drop(file);

        let hash1 = hash_file(&file_path, &HashAlgorithm::Sha256).unwrap();
        let hash2 = hash_file(&file_path, &HashAlgorithm::Sha256).unwrap();
        assert_eq!(hash1.hex, hash2.hex);

        std::fs::remove_file(&file_path).ok();
        std::fs::remove_dir(&dir).ok();
    }

    #[test]
    fn test_different_content_different_hash() {
        let dir = std::env::temp_dir().join("sincronia_test_hash_diff");
        std::fs::create_dir_all(&dir).unwrap();
        let file_a = dir.join("a.bin");
        let file_b = dir.join("b.bin");

        File::create(&file_a)
            .unwrap()
            .write_all(b"Content A")
            .unwrap();
        File::create(&file_b)
            .unwrap()
            .write_all(b"Content B")
            .unwrap();

        let hash_a = hash_file(&file_a, &HashAlgorithm::Blake3).unwrap();
        let hash_b = hash_file(&file_b, &HashAlgorithm::Blake3).unwrap();
        assert_ne!(hash_a.hex, hash_b.hex);

        std::fs::remove_file(&file_a).ok();
        std::fs::remove_file(&file_b).ok();
        std::fs::remove_dir(&dir).ok();
    }
}
