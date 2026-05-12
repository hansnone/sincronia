// sincronia/src/metadata.rs
//
// Aplicación de metadatos (atributos y timestamps) a archivos copiados.
// Usa Windows APIs para SetFileTime y SetFileAttributesW.

use crate::config::MetadataConfig;
use crate::errors::SincroniaError;
use crate::scanner::FileEntry;
use std::path::Path;
use tracing::{debug, trace, warn};

#[cfg(windows)]
use std::os::windows::fs::MetadataExt;

/// Aplica metadatos del archivo original al archivo de destino
pub fn apply_metadata(
    destination_path: &Path,
    original: &FileEntry,
    config: &MetadataConfig,
) -> Result<(), SincroniaError> {
    debug!(
        "Aplicando metadatos a: {}",
        destination_path.display()
    );

    // Aplicar timestamps
    apply_timestamps(destination_path, original, config)?;

    // Aplicar atributos de archivo
    if config.preserve_file_attributes {
        apply_attributes(destination_path, original.attributes)?;
    }

    Ok(())
}

/// Aplica timestamps (creación, modificación, acceso) vía Windows API
#[cfg(windows)]
fn apply_timestamps(
    destination_path: &Path,
    original: &FileEntry,
    config: &MetadataConfig,
) -> Result<(), SincroniaError> {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, SetFileTime, FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_READ,
        FILE_WRITE_ATTRIBUTES, OPEN_EXISTING,
    };
    use windows::Win32::Foundation::FILETIME;
    use windows::core::PCWSTR;

    let path_wide: Vec<u16> = destination_path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    // SAFETY: Abrimos el archivo con FILE_WRITE_ATTRIBUTES para modificar timestamps.
    // FILE_FLAG_BACKUP_SEMANTICS permite abrir directorios también.
    // El handle se cierra automáticamente al salir del scope.
    let handle = unsafe {
        CreateFileW(
            PCWSTR::from_raw(path_wide.as_ptr()),
            FILE_WRITE_ATTRIBUTES.0,
            FILE_SHARE_READ,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
    }
    .map_err(|e| SincroniaError::Metadata {
        path: destination_path.to_path_buf(),
        message: format!("CreateFileW para timestamps: {}", e),
    })?;

    // Convertir SystemTime a FILETIME
    let creation_time = if config.preserve_creation_time {
        original.creation_time.map(|t| system_time_to_filetime(t))
    } else {
        None
    };

    let last_write_time = if config.preserve_last_write_time {
        Some(system_time_to_filetime(original.last_write_time))
    } else {
        None
    };

    let last_access_time = if config.preserve_last_access_time {
        original.last_access_time.map(|t| system_time_to_filetime(t))
    } else {
        None
    };

    // SAFETY: SetFileTime establece timestamps del archivo.
    // El handle es válido (recién abierto), y los punteros a FILETIME
    // son Option → pasamos null para los que no queremos modificar.
    let result = unsafe {
        SetFileTime(
            handle,
            creation_time.as_ref().map(|t| t as *const FILETIME),
            last_access_time.as_ref().map(|t| t as *const FILETIME),
            last_write_time.as_ref().map(|t| t as *const FILETIME),
        )
    };

    // Cerrar handle explícitamente
    unsafe {
        windows::Win32::Foundation::CloseHandle(handle).ok();
    }

    match result {
        Ok(()) => {
            trace!("Timestamps aplicados a: {}", destination_path.display());
            Ok(())
        }
        Err(e) => {
            warn!(
                "Error al aplicar timestamps a '{}': {}",
                destination_path.display(),
                e
            );
            Err(SincroniaError::Metadata {
                path: destination_path.to_path_buf(),
                message: format!("SetFileTime: {}", e),
            })
        }
    }
}

#[cfg(not(windows))]
fn apply_timestamps(
    destination_path: &Path,
    _original: &FileEntry,
    _config: &MetadataConfig,
) -> Result<(), SincroniaError> {
    warn!(
        "Aplicación de timestamps no soportada en esta plataforma: {}",
        destination_path.display()
    );
    Ok(())
}

/// Aplica atributos de archivo (readonly, hidden, system, archive)
#[cfg(windows)]
fn apply_attributes(
    destination_path: &Path,
    attributes: u32,
) -> Result<(), SincroniaError> {
    use windows::Win32::Storage::FileSystem::SetFileAttributesW;
    use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
    use windows::core::PCWSTR;

    if attributes == 0 {
        return Ok(());
    }

    let path_wide: Vec<u16> = destination_path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    // SAFETY: SetFileAttributesW establece los atributos del archivo.
    // Solo modifica atributos de archivo estándar (readonly, hidden, etc.).
    let result = unsafe {
        SetFileAttributesW(
            PCWSTR::from_raw(path_wide.as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(attributes),
        )
    };

    match result {
        Ok(()) => {
            trace!(
                "Atributos aplicados a {}: 0x{:X}",
                destination_path.display(),
                attributes
            );
            Ok(())
        }
        Err(e) => {
            warn!(
                "Error al aplicar atributos a '{}': {}",
                destination_path.display(),
                e
            );
            Err(SincroniaError::Metadata {
                path: destination_path.to_path_buf(),
                message: format!("SetFileAttributesW: {}", e),
            })
        }
    }
}

#[cfg(not(windows))]
fn apply_attributes(
    _destination_path: &Path,
    _attributes: u32,
) -> Result<(), SincroniaError> {
    Ok(())
}

/// Convierte SystemTime a FILETIME de Windows
#[cfg(windows)]
fn system_time_to_filetime(time: std::time::SystemTime) -> windows::Win32::Foundation::FILETIME {
    use windows::Win32::Foundation::FILETIME;

    // FILETIME cuenta intervalos de 100ns desde 1601-01-01
    // UNIX_EPOCH es 1970-01-01
    // Diferencia: 11644473600 segundos
    const UNIX_TO_FILETIME_OFFSET: u64 = 11_644_473_600;
    const HUNDRED_NS_PER_SECOND: u64 = 10_000_000;

    let duration = time
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();

    let filetime_value =
        (duration.as_secs() + UNIX_TO_FILETIME_OFFSET) * HUNDRED_NS_PER_SECOND
            + (duration.subsec_nanos() as u64) / 100;

    FILETIME {
        dwLowDateTime: filetime_value as u32,
        dwHighDateTime: (filetime_value >> 32) as u32,
    }
}
