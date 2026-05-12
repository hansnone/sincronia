// sincronia/src/planner.rs
//
// Planificador de trabajos de copia.
// Genera CopyJobs a partir de archivos estables, calcula rutas de destino,
// y ordena por prioridad.

use crate::scanner::FileEntry;
use std::path::{Path, PathBuf};
use tracing::debug;

/// Trabajo de copia individual
#[derive(Debug, Clone)]
pub struct CopyJob {
    /// Ruta absoluta del archivo origen
    pub source_path: PathBuf,
    /// Ruta relativa al directorio origen
    pub relative_path: PathBuf,
    /// Ruta absoluta del archivo destino final
    pub destination_path: PathBuf,
    /// Ruta absoluta del archivo temporal de destino (.partial)
    pub temp_destination_path: PathBuf,
    /// Tamaño del archivo en bytes
    pub size: u64,
    /// Metadatos del archivo original (para aplicar después)
    pub original_entry: FileEntry,
    /// Si el archivo es "grande" según el umbral configurado
    pub is_large_file: bool,
}

/// Genera trabajos de copia a partir de archivos estables
pub fn plan_copy_jobs(
    stable_files: &[FileEntry],
    destination_base: &Path,
    temp_extension: &str,
    large_file_threshold_bytes: u64,
) -> Vec<CopyJob> {
    let mut jobs: Vec<CopyJob> = stable_files
        .iter()
        .map(|entry| {
            let destination_path = destination_base.join(&entry.relative_path);
            let temp_destination_path = PathBuf::from(format!(
                "{}{}",
                destination_path.to_string_lossy(),
                temp_extension
            ));

            CopyJob {
                source_path: entry.absolute_path.clone(),
                relative_path: entry.relative_path.clone(),
                destination_path,
                temp_destination_path,
                size: entry.size,
                original_entry: entry.clone(),
                is_large_file: entry.size >= large_file_threshold_bytes,
            }
        })
        .collect();

    // Ordenar: archivos pequeños primero para maximizar throughput
    // con cargas mixtas (muchos pequeños se completan rápido mientras
    // pocos workers procesan los grandes)
    jobs.sort_by(|a, b| a.size.cmp(&b.size));

    let large_count = jobs.iter().filter(|j| j.is_large_file).count();
    debug!(
        "Planificador: {} trabajos generados ({} grandes, {} pequeños)",
        jobs.len(),
        large_count,
        jobs.len() - large_count
    );

    jobs
}

/// Asegura que existan los directorios necesarios en destino
pub fn ensure_destination_directories(
    directories: &[PathBuf],
    destination_base: &Path,
) -> Result<(), std::io::Error> {
    for dir in directories {
        let dest_dir = destination_base.join(dir);
        if !dest_dir.exists() {
            debug!("Creando directorio en destino: {}", dest_dir.display());
            std::fs::create_dir_all(&dest_dir)?;
        }
    }
    Ok(())
}

/// Limpia directorios vacíos del origen después del procesamiento
pub fn remove_empty_source_directories(
    base_path: &Path,
    directories: &[PathBuf],
) -> u32 {
    let mut removed = 0;

    // Procesar en orden inverso (más profundo primero)
    let mut sorted_dirs = directories.to_vec();
    sorted_dirs.sort_by(|a, b| b.components().count().cmp(&a.components().count()));

    for dir in &sorted_dirs {
        let full_path = base_path.join(dir);
        if full_path.exists() {
            // Comprobar si está vacío
            match std::fs::read_dir(&full_path) {
                Ok(mut entries) => {
                    if entries.next().is_none() {
                        match std::fs::remove_dir(&full_path) {
                            Ok(()) => {
                                debug!("Directorio vacío eliminado: {}", full_path.display());
                                removed += 1;
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "No se pudo eliminar directorio vacío '{}': {}",
                                    full_path.display(),
                                    e
                                );
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "Error leyendo directorio '{}': {}",
                        full_path.display(),
                        e
                    );
                }
            }
        }
    }

    if removed > 0 {
        debug!("{} directorios vacíos eliminados del origen", removed);
    }

    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    fn make_entry(name: &str, size: u64) -> FileEntry {
        FileEntry {
            relative_path: PathBuf::from(name),
            absolute_path: PathBuf::from(format!("C:\\source\\{}", name)),
            size,
            last_write_time: SystemTime::now(),
            creation_time: None,
            last_access_time: None,
            attributes: 0,
        }
    }

    #[test]
    fn test_plan_generates_correct_paths() {
        let files = vec![make_entry("subdir\\file.txt", 1000)];
        let dest = Path::new("R:\\Backup");
        let jobs = plan_copy_jobs(&files, dest, ".partial", 1024 * 1024 * 1024);

        assert_eq!(jobs.len(), 1);
        assert_eq!(
            jobs[0].destination_path,
            PathBuf::from("R:\\Backup\\subdir\\file.txt")
        );
        assert_eq!(
            jobs[0].temp_destination_path,
            PathBuf::from("R:\\Backup\\subdir\\file.txt.partial")
        );
    }

    #[test]
    fn test_plan_sorts_small_first() {
        let files = vec![
            make_entry("large.bin", 10_000_000),
            make_entry("tiny.txt", 100),
            make_entry("medium.doc", 500_000),
        ];
        let jobs = plan_copy_jobs(&files, Path::new("R:\\"), ".partial", 5_000_000);

        assert_eq!(jobs[0].size, 100);
        assert_eq!(jobs[1].size, 500_000);
        assert_eq!(jobs[2].size, 10_000_000);
        assert!(jobs[2].is_large_file);
        assert!(!jobs[0].is_large_file);
    }
}
