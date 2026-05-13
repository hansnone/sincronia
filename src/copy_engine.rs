// sincronia/src/copy_engine.rs
//
// Motor de copia por bloques con buffer preasignado.
// Escribe a archivo temporal, hace flush/sync, y prepara para verificación.
// Usa FILE_FLAG_SEQUENTIAL_SCAN para hint de prefetch al OS.

use crate::errors::SincroniaError;
use crate::planner::CopyJob;
use std::io::{Read, Write};
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};
use tracing::{debug, trace, warn};

/// Resultado de la copia de un archivo
#[derive(Debug, Clone)]
pub struct CopyResult {
    /// Bytes copiados
    pub bytes_copied: u64,
    /// Duración de la copia en milisegundos
    pub copy_duration_ms: u64,
    /// Velocidad media en MB/s
    pub average_speed_mbps: f64,
}

/// Copia un archivo del origen al destino temporal usando el buffer proporcionado.
/// El buffer se reutiliza entre archivos para evitar allocations.
pub fn copy_file_buffered(
    job: &CopyJob,
    buffer: &mut Vec<u8>,
) -> Result<CopyResult, SincroniaError> {
    let source = &job.source_path;
    let temp_dest = &job.temp_destination_path;

    debug!(
        "Copiando: {} → {} ({} bytes)",
        source.display(),
        temp_dest.display(),
        job.size
    );

    // Asegurar que el directorio destino existe
    if let Some(parent) = temp_dest.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent).map_err(|e| SincroniaError::Copy {
                path: temp_dest.clone(),
                message: format!("No se pudo crear directorio destino: {}", e),
            })?;
        }
    }

    let start = Instant::now();

    // Abrir origen con hint de lectura secuencial
    let mut reader = open_source_file(source)?;

    // Crear archivo temporal de destino
    let mut writer = create_temp_destination(temp_dest)?;

    let mut bytes_copied: u64 = 0;

    // Loop de copia por bloques
    loop {
        let bytes_read = reader.read(buffer).map_err(|e| SincroniaError::Copy {
            path: source.clone(),
            message: format!("Error de lectura en offset {}: {}", bytes_copied, e),
        })?;

        if bytes_read == 0 {
            break;
        }

        writer
            .write_all(&buffer[..bytes_read])
            .map_err(|e| SincroniaError::Copy {
                path: temp_dest.clone(),
                message: format!("Error de escritura en offset {}: {}", bytes_copied, e),
            })?;

        bytes_copied += bytes_read as u64;
    }

    // Flush explícito para asegurar que los datos llegan al disco
    writer.flush().map_err(|e| SincroniaError::Copy {
        path: temp_dest.clone(),
        message: format!("Error en flush: {}", e),
    })?;

    // Sync para forzar escritura física (especialmente importante sobre SMB)
    sync_file(&writer)?;

    // Cerrar handles de inmediato: sobre SMB (p. ej. smbd en macOS), el cierre
    // asíncrono puede retrasar el `rename` del `.partial`; liberar aquí reduce la ventana.
    drop(writer);
    drop(reader);

    let duration = start.elapsed();
    let duration_ms = duration.as_millis() as u64;
    let speed_mbps = if duration_ms > 0 {
        (bytes_copied as f64 / 1_048_576.0) / (duration_ms as f64 / 1000.0)
    } else {
        0.0
    };

    debug!(
        "Copia completada: {} bytes en {:.1}ms ({:.1} MB/s)",
        bytes_copied, duration_ms, speed_mbps
    );

    Ok(CopyResult {
        bytes_copied,
        copy_duration_ms: duration_ms,
        average_speed_mbps: speed_mbps,
    })
}

/// Abre el archivo origen con hints de lectura secuencial
#[cfg(windows)]
fn open_source_file(path: &Path) -> Result<std::fs::File, SincroniaError> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows::Win32::Storage::FileSystem::FILE_FLAG_SEQUENTIAL_SCAN;

    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_SEQUENTIAL_SCAN.0)
        .open(path)
        .map_err(|e| SincroniaError::Copy {
            path: path.to_path_buf(),
            message: format!("No se pudo abrir origen: {}", e),
        })
}

#[cfg(not(windows))]
fn open_source_file(path: &Path) -> Result<std::fs::File, SincroniaError> {
    std::fs::File::open(path).map_err(|e| SincroniaError::Copy {
        path: path.to_path_buf(),
        message: format!("No se pudo abrir origen: {}", e),
    })
}

/// Crea el archivo temporal de destino
fn create_temp_destination(path: &Path) -> Result<std::fs::File, SincroniaError> {
    // Si ya existe un .partial previo (copia interrumpida), eliminarlo
    if path.exists() {
        warn!(
            "Eliminando archivo temporal previo: {}",
            path.display()
        );
        std::fs::remove_file(path).map_err(|e| SincroniaError::Copy {
            path: path.to_path_buf(),
            message: format!("No se pudo eliminar temporal previo: {}", e),
        })?;
    }

    std::fs::File::create(path).map_err(|e| SincroniaError::Copy {
        path: path.to_path_buf(),
        message: format!("No se pudo crear archivo temporal: {}", e),
    })
}

/// Fuerza sincronización a disco del archivo
#[cfg(windows)]
fn sync_file(file: &std::fs::File) -> Result<(), SincroniaError> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::FlushFileBuffers;

    let handle = HANDLE(file.as_raw_handle());

    // SAFETY: FlushFileBuffers fuerza la escritura de buffers pendientes al disco.
    // El handle es válido porque el File está vivo en este scope.
    unsafe {
        FlushFileBuffers(handle).map_err(|e| SincroniaError::Copy {
            path: std::path::PathBuf::from("<sync>"),
            message: format!("FlushFileBuffers falló: {}", e),
        })?;
    }

    trace!("FlushFileBuffers completado");
    Ok(())
}

#[cfg(not(windows))]
fn sync_file(file: &std::fs::File) -> Result<(), SincroniaError> {
    file.sync_all().map_err(|e| SincroniaError::Copy {
        path: std::path::PathBuf::from("<sync>"),
        message: format!("sync_all falló: {}", e),
    })
}

/// Renombra el archivo temporal al nombre final (operación atómica en NTFS/ReFS).
/// Reintenta el `rename` por latencia SMB: en destinos macOS/APFS el servidor puede
/// tardar unos ms en liberar el bloqueo tras cerrar el handle del cliente.
pub fn finalize_copy(temp_path: &Path, final_path: &Path) -> Result<(), SincroniaError> {
    debug!(
        "Finalizando: {} → {}",
        temp_path.display(),
        final_path.display()
    );

    const MAX_ATTEMPTS: u32 = 5;
    const RETRY_DELAY: Duration = Duration::from_millis(200);

    let mut last_err = None::<std::io::Error>;

    for attempt in 1..=MAX_ATTEMPTS {
        match std::fs::rename(temp_path, final_path) {
            Ok(()) => return Ok(()),
            Err(e) => {
                if attempt < MAX_ATTEMPTS {
                    trace!(
                        "rename intento {}/{} falló, reintentando tras {:?}: {}",
                        attempt,
                        MAX_ATTEMPTS,
                        RETRY_DELAY,
                        e
                    );
                    thread::sleep(RETRY_DELAY);
                }
                last_err = Some(e);
            }
        }
    }

    let e = last_err.expect("al menos un intento de rename");
    Err(SincroniaError::Copy {
        path: final_path.to_path_buf(),
        message: format!(
            "Error al renombrar temporal '{}' → '{}' tras {} intentos: {}",
            temp_path.display(),
            final_path.display(),
            MAX_ATTEMPTS,
            e
        ),
    })
}

/// Limpia un archivo temporal huérfano (de una copia fallida)
pub fn cleanup_temp_file(temp_path: &Path) {
    if temp_path.exists() {
        match std::fs::remove_file(temp_path) {
            Ok(()) => debug!("Temporal limpiado: {}", temp_path.display()),
            Err(e) => warn!(
                "No se pudo limpiar temporal '{}': {}",
                temp_path.display(),
                e
            ),
        }
    }
}
