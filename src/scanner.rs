// sincronia/src/scanner.rs
//
// Recorrido recursivo del directorio origen.
// Soporta rutas largas (\\?\), detecta symlinks/junctions, aplica exclusiones.

use crate::exclusions::ExclusionFilter;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use tracing::{debug, trace, warn};

/// Entrada de archivo detectada por el scanner
#[derive(Debug, Clone)]
pub struct FileEntry {
    /// Ruta relativa al directorio origen
    pub relative_path: PathBuf,
    /// Ruta absoluta del archivo
    pub absolute_path: PathBuf,
    /// Tamaño en bytes
    pub size: u64,
    /// Fecha de última escritura
    pub last_write_time: SystemTime,
    /// Fecha de creación (si disponible)
    pub creation_time: Option<SystemTime>,
    /// Fecha de último acceso (si disponible)
    pub last_access_time: Option<SystemTime>,
    /// Atributos del archivo (raw u32 en Windows)
    pub attributes: u32,
}

/// Resultado del escaneo
#[derive(Debug)]
pub struct ScanResult {
    /// Archivos detectados
    pub files: Vec<FileEntry>,
    /// Directorios encontrados (rutas relativas)
    pub directories: Vec<PathBuf>,
    /// Archivos excluidos (conteo)
    pub excluded_count: u64,
    /// Symlinks/junctions ignorados (conteo)
    pub symlink_count: u64,
    /// Errores de lectura (conteo)
    pub error_count: u64,
}

/// Normaliza una ruta para soportar rutas largas en Windows (prefijo \\?\)
pub fn normalize_long_path(path: &Path) -> PathBuf {
    let path_str = path.to_string_lossy();

    // Si ya tiene prefijo de ruta larga, no modificar
    if path_str.starts_with("\\\\?\\") {
        return path.to_path_buf();
    }

    // Para rutas UNC (\\server\share), usar \\?\UNC\server\share
    if path_str.starts_with("\\\\") {
        let without_prefix = &path_str[2..]; // Quitar las dos barras iniciales
        return PathBuf::from(format!("\\\\?\\UNC\\{}", without_prefix));
    }

    // Para rutas normales (C:\...), usar \\?\C:\...
    if path_str.len() >= 2 && path_str.chars().nth(1) == Some(':') {
        return PathBuf::from(format!("\\\\?\\{}", path_str));
    }

    // Para rutas relativas, intentar obtener la ruta absoluta
    match path.canonicalize() {
        Ok(canonical) => {
            let canonical_str = canonical.to_string_lossy();
            if !canonical_str.starts_with("\\\\?\\") {
                PathBuf::from(format!("\\\\?\\{}", canonical_str))
            } else {
                canonical
            }
        }
        Err(_) => path.to_path_buf(),
    }
}

/// Escanea recursivamente un directorio aplicando filtros
pub fn scan_directory(
    base_path: &Path,
    filter: &ExclusionFilter,
    ignore_symlinks: bool,
    ignore_junctions: bool,
) -> ScanResult {
    let mut result = ScanResult {
        files: Vec::new(),
        directories: Vec::new(),
        excluded_count: 0,
        symlink_count: 0,
        error_count: 0,
    };

    let normalized_base = normalize_long_path(base_path);
    debug!(
        "Iniciando escaneo de: {} (normalizado: {})",
        base_path.display(),
        normalized_base.display()
    );

    scan_recursive(
        &normalized_base,
        &normalized_base,
        filter,
        ignore_symlinks,
        ignore_junctions,
        &mut result,
    );

    debug!(
        "Escaneo completado: {} archivos, {} directorios, {} excluidos, {} symlinks, {} errores",
        result.files.len(),
        result.directories.len(),
        result.excluded_count,
        result.symlink_count,
        result.error_count
    );

    result
}

fn scan_recursive(
    current_path: &Path,
    base_path: &Path,
    filter: &ExclusionFilter,
    ignore_symlinks: bool,
    ignore_junctions: bool,
    result: &mut ScanResult,
) {
    let entries = match fs::read_dir(current_path) {
        Ok(entries) => entries,
        Err(e) => {
            warn!(
                "Error al leer directorio '{}': {}",
                current_path.display(),
                e
            );
            result.error_count += 1;
            return;
        }
    };

    for entry_result in entries {
        let entry = match entry_result {
            Ok(entry) => entry,
            Err(e) => {
                warn!("Error leyendo entrada de directorio: {}", e);
                result.error_count += 1;
                continue;
            }
        };

        let path = entry.path();
        let file_name = match entry.file_name().to_str() {
            Some(name) => name.to_string(),
            None => {
                warn!(
                    "Nombre de archivo no válido UTF-8: {:?}",
                    entry.file_name()
                );
                // Usar to_string_lossy para intentar continuar
                entry.file_name().to_string_lossy().to_string()
            }
        };

        // Obtener metadatos sin seguir symlinks
        let metadata = match fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) => {
                warn!("Error obteniendo metadatos de '{}': {}", path.display(), e);
                result.error_count += 1;
                continue;
            }
        };

        // Detectar symlinks y junctions
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            if ignore_symlinks {
                trace!("Ignorando symlink: {}", path.display());
                result.symlink_count += 1;
                continue;
            }
        }

        // En Windows, los junctions tienen FILE_ATTRIBUTE_REPARSE_POINT
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            let attrs = metadata.file_attributes();
            const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
            if attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0 && ignore_junctions {
                trace!("Ignorando junction/reparse point: {}", path.display());
                result.symlink_count += 1;
                continue;
            }
        }

        let relative_path = match path.strip_prefix(base_path) {
            Ok(rel) => rel.to_path_buf(),
            Err(_) => {
                warn!(
                    "No se pudo obtener ruta relativa para: {}",
                    path.display()
                );
                result.error_count += 1;
                continue;
            }
        };

        if metadata.is_dir() {
            // Comprobar exclusión de directorio
            if filter.is_directory_excluded(&file_name) {
                trace!("Directorio excluido: {}", file_name);
                result.excluded_count += 1;
                continue;
            }

            result.directories.push(relative_path.clone());

            // Recursión
            scan_recursive(
                &path,
                base_path,
                filter,
                ignore_symlinks,
                ignore_junctions,
                result,
            );
        } else if metadata.is_file() {
            // Comprobar exclusión de archivo
            if filter.is_file_excluded(&file_name) {
                trace!("Archivo excluido: {}", file_name);
                result.excluded_count += 1;
                continue;
            }

            let last_write_time = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let creation_time = metadata.created().ok();
            let last_access_time = metadata.accessed().ok();

            #[cfg(windows)]
            let attributes = {
                use std::os::windows::fs::MetadataExt;
                metadata.file_attributes()
            };
            #[cfg(not(windows))]
            let attributes = 0u32;

            result.files.push(FileEntry {
                relative_path,
                absolute_path: path.clone(),
                size: metadata.len(),
                last_write_time,
                creation_time,
                last_access_time,
                attributes,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_long_path_drive() {
        let path = Path::new("C:\\Users\\test\\file.txt");
        let normalized = normalize_long_path(path);
        assert!(normalized.to_string_lossy().starts_with("\\\\?\\"));
    }

    #[test]
    fn test_normalize_long_path_unc() {
        let path = Path::new("\\\\server\\share\\folder");
        let normalized = normalize_long_path(path);
        assert!(normalized.to_string_lossy().starts_with("\\\\?\\UNC\\"));
    }

    #[test]
    fn test_normalize_long_path_already_prefixed() {
        let path = Path::new("\\\\?\\C:\\Users\\test");
        let normalized = normalize_long_path(path);
        assert_eq!(normalized, path);
    }
}
