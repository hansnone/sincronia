// sincronia/src/stability.rs
//
// Detector de estabilidad de archivos.
// Un archivo es estable si su tamaño y LastWriteTime no cambian
// durante minimum_file_stable_seconds.

use crate::scanner::FileEntry;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Instant, SystemTime};
use tracing::{debug, trace};

/// Registro de observación de un archivo
#[derive(Debug, Clone)]
struct FileObservation {
    /// Tamaño en la primera observación
    size: u64,
    /// LastWriteTime en la primera observación
    last_write_time: SystemTime,
    /// Instante de la primera observación estable
    first_seen_stable: Instant,
}

/// Clave de caché para archivos ya respaldados.
/// Combina ruta relativa + tamaño + mtime para detectar si el archivo cambió.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct BackedUpKey {
    relative_path: PathBuf,
    size: u64,
    /// Epoch en segundos del last_write_time (para poder hacer Hash)
    mtime_epoch_secs: u64,
}

impl BackedUpKey {
    fn from_entry(entry: &FileEntry) -> Self {
        let mtime_epoch_secs = entry
            .last_write_time
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Self {
            relative_path: entry.relative_path.clone(),
            size: entry.size,
            mtime_epoch_secs,
        }
    }
}

/// Detector de estabilidad basado en observaciones sucesivas
pub struct StabilityChecker {
    /// Mapa de archivos bajo observación
    observations: HashMap<PathBuf, FileObservation>,
    /// Segundos requeridos sin cambios
    stable_seconds: u64,
    /// Caché de archivos ya respaldados exitosamente.
    /// Evita re-hashear archivos idénticos en cada ciclo de escaneo
    /// (especialmente importante en modo backup_append_only donde
    /// los archivos permanecen en origen después de respaldarlos).
    backed_up_cache: HashSet<BackedUpKey>,
}

impl StabilityChecker {
    pub fn new(minimum_file_stable_seconds: u64) -> Self {
        debug!(
            "StabilityChecker: archivos deben ser estables durante {} segundos",
            minimum_file_stable_seconds
        );
        Self {
            observations: HashMap::new(),
            stable_seconds: minimum_file_stable_seconds,
            backed_up_cache: HashSet::new(),
        }
    }

    /// Evalúa una lista de archivos y devuelve los que son estables.
    /// Los inestables se mantienen en observación.
    pub fn evaluate(&mut self, files: &[FileEntry]) -> (Vec<FileEntry>, Vec<FileEntry>) {
        let mut stable = Vec::new();
        let mut unstable = Vec::new();
        let now = Instant::now();

        // Registrar qué rutas están en el scan actual
        let current_paths: std::collections::HashSet<_> =
            files.iter().map(|f| f.relative_path.clone()).collect();

        for file in files {
            let key = &file.relative_path;

            // Si ya está en la caché de respaldados y no cambió, saltarlo
            let backed_up_key = BackedUpKey::from_entry(file);
            if self.backed_up_cache.contains(&backed_up_key) {
                trace!(
                    "Archivo en caché de respaldados (saltando): {} (size: {})",
                    key.display(),
                    file.size
                );
                // No lo contamos como estable ni inestable — simplemente lo ignoramos
                continue;
            }

            match self.observations.get(key) {
                Some(obs) => {
                    // Si el tamaño o mtime cambiaron, reiniciar observación
                    if obs.size != file.size || obs.last_write_time != file.last_write_time {
                        trace!(
                            "Archivo inestable (cambió): {} (size: {} → {}, mtime cambió: {})",
                            key.display(),
                            obs.size,
                            file.size,
                            obs.last_write_time != file.last_write_time
                        );
                        self.observations.insert(
                            key.clone(),
                            FileObservation {
                                size: file.size,
                                last_write_time: file.last_write_time,
                                first_seen_stable: now,
                            },
                        );
                        unstable.push(file.clone());
                    } else {
                        // Tamaño y mtime no cambiaron — verificar tiempo transcurrido
                        let elapsed = now.duration_since(obs.first_seen_stable);
                        if elapsed.as_secs() >= self.stable_seconds {
                            trace!(
                                "Archivo estable: {} ({:.1}s sin cambios)",
                                key.display(),
                                elapsed.as_secs_f64()
                            );
                            stable.push(file.clone());
                        } else {
                            trace!(
                                "Archivo en observación: {} ({:.1}s / {}s)",
                                key.display(),
                                elapsed.as_secs_f64(),
                                self.stable_seconds
                            );
                            unstable.push(file.clone());
                        }
                    }
                }
                None => {
                    // Primera observación — registrar
                    trace!(
                        "Primera observación: {} (size: {}, esperando {}s)",
                        key.display(),
                        file.size,
                        self.stable_seconds
                    );
                    self.observations.insert(
                        key.clone(),
                        FileObservation {
                            size: file.size,
                            last_write_time: file.last_write_time,
                            first_seen_stable: now,
                        },
                    );
                    unstable.push(file.clone());
                }
            }
        }

        // Limpiar observaciones de archivos que ya no existen en el scan
        self.observations
            .retain(|k, _| current_paths.contains(k));

        debug!(
            "Estabilidad: {} estables, {} inestables, {} en observación, {} en caché de respaldados",
            stable.len(),
            unstable.len(),
            self.observations.len(),
            self.backed_up_cache.len()
        );

        (stable, unstable)
    }

    /// Marca un archivo como procesado (elimina de observación)
    pub fn mark_processed(&mut self, relative_path: &PathBuf) {
        self.observations.remove(relative_path);
    }

    /// Marca un archivo como ya respaldado exitosamente.
    /// Los archivos en esta caché se omiten en futuros escaneos,
    /// evitando el costoso re-hasheo de archivos que no han cambiado.
    pub fn mark_backed_up(&mut self, entry: &FileEntry) {
        let key = BackedUpKey::from_entry(entry);
        self.backed_up_cache.insert(key);
        // También eliminar de observaciones activas
        self.observations.remove(&entry.relative_path);
    }

    /// Número de archivos bajo observación
    pub fn pending_count(&self) -> usize {
        self.observations.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn make_entry(name: &str, size: u64) -> FileEntry {
        FileEntry {
            relative_path: PathBuf::from(name),
            absolute_path: PathBuf::from(format!("C:\\test\\{}", name)),
            size,
            last_write_time: SystemTime::now(),
            creation_time: None,
            last_access_time: None,
            attributes: 0,
        }
    }

    #[test]
    fn test_new_file_is_unstable() {
        let mut checker = StabilityChecker::new(60);
        let files = vec![make_entry("file.txt", 1000)];
        let (stable, unstable) = checker.evaluate(&files);
        assert_eq!(stable.len(), 0);
        assert_eq!(unstable.len(), 1);
    }

    #[test]
    fn test_file_becomes_stable_with_zero_delay() {
        let mut checker = StabilityChecker::new(0);
        let files = vec![make_entry("file.txt", 1000)];

        // Primera observación
        let (stable, _) = checker.evaluate(&files);
        assert_eq!(stable.len(), 0); // Primera vez siempre es inestable

        // Segunda observación — con stable_seconds=0, debe ser estable
        let (stable, _) = checker.evaluate(&files);
        assert_eq!(stable.len(), 1);
    }

    #[test]
    fn test_cleanup_removed_files() {
        let mut checker = StabilityChecker::new(60);
        let files = vec![make_entry("file.txt", 1000)];
        checker.evaluate(&files);
        assert_eq!(checker.pending_count(), 1);

        // Escaneo vacío — el archivo desapareció
        let empty: Vec<FileEntry> = vec![];
        checker.evaluate(&empty);
        assert_eq!(checker.pending_count(), 0);
    }
}
