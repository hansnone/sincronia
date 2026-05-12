// sincronia/src/windows_nas.rs
//
// Gestión de conexión al NAS vía APIs Windows WNet y fallback net use.
// Prioriza WNetAddConnection3W (seguro) sobre net use (línea de comandos).

use crate::config::NasConfig;
use crate::errors::SincroniaError;
use std::process::Command;
use tracing::{debug, error, info, warn};

#[cfg(windows)]
use windows::core::PCWSTR;
#[cfg(windows)]
use windows::Win32::Foundation::WIN32_ERROR;
#[cfg(windows)]
use windows::Win32::NetworkManagement::WNet::{
    WNetAddConnection2W, WNetCancelConnection2W, WNetGetConnectionW, NETRESOURCEW,
    NET_CONNECT_FLAGS, RESOURCETYPE_DISK,
};

/// Error code constants
#[cfg(windows)]
const NO_ERROR: u32 = 0;
#[cfg(windows)]
const ERROR_NOT_CONNECTED: u32 = 2250;
#[cfg(windows)]
const ERROR_NO_NETWORK: u32 = 1222;

/// Resultado de la validación de la unidad
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriveValidation {
    /// R: apunta correctamente al UNC primario
    ValidPrimary,
    /// R: apunta al UNC de fallback por IP (solo si allow_ip_fallback = true)
    ValidFallbackIp,
    /// R: no está montada
    NotMounted,
    /// R: apunta a otro recurso
    PointsElsewhere { current_target: String },
    /// Error al consultar
    Error(String),
}

/// Consulta a qué recurso UNC apunta una letra de unidad.
/// Retorna None si la unidad no está mapeada.
#[cfg(windows)]
pub fn get_drive_connection(drive_letter: &str) -> Result<Option<String>, SincroniaError> {
    let local_wide: Vec<u16> = drive_letter.encode_utf16().chain(std::iter::once(0)).collect();
    let mut buffer = [0u16; 1024];
    let mut buffer_size = buffer.len() as u32;

    // SAFETY: Llamamos a WNetGetConnectionW con un buffer preasignado.
    // El buffer tiene tamaño suficiente para cualquier ruta UNC válida.
    // buffer_size se pasa por referencia y se actualiza con el tamaño real.
    // SAFETY: buffer is valid for the duration of the call
    let result: WIN32_ERROR = unsafe {
        WNetGetConnectionW(
            PCWSTR::from_raw(local_wide.as_ptr()),
            Some(windows::core::PWSTR::from_raw(buffer.as_mut_ptr())),
            &mut buffer_size,
        )
    };

    if result.0 == NO_ERROR {
        let unc = String::from_utf16_lossy(&buffer[..buffer_size as usize])
            .trim_end_matches('\0')
            .to_string();
        debug!("Unidad {} apunta a: {}", drive_letter, unc);
        Ok(Some(unc))
    } else if result.0 == ERROR_NOT_CONNECTED || result.0 == ERROR_NO_NETWORK {
        debug!("Unidad {} no está conectada", drive_letter);
        Ok(None)
    } else {
        Err(SincroniaError::WindowsApi {
            message: format!("WNetGetConnectionW falló para {}", drive_letter),
            code: result.0,
        })
    }
}

#[cfg(not(windows))]
pub fn get_drive_connection(_drive_letter: &str) -> Result<Option<String>, SincroniaError> {
    Ok(None)
}

/// Valida si la unidad requerida está correctamente mapeada al NAS
pub fn validate_drive(config: &NasConfig) -> DriveValidation {
    let letter = &config.required_drive_letter;

    match get_drive_connection(letter) {
        Ok(None) => {
            info!("Unidad {} no está montada", letter);
            DriveValidation::NotMounted
        }
        Ok(Some(current_unc)) => {
            let current_upper = current_unc.to_uppercase();
            let primary_upper = config.primary_unc_path.to_uppercase();

            if current_upper == primary_upper {
                info!("Unidad {} correctamente mapeada a {}", letter, current_unc);
                DriveValidation::ValidPrimary
            } else if config.allow_ip_fallback {
                let fallback_upper = config.fallback_unc_path_by_ip.to_uppercase();
                if current_upper == fallback_upper {
                    info!(
                        "Unidad {} mapeada al fallback por IP: {}",
                        letter, current_unc
                    );
                    DriveValidation::ValidFallbackIp
                } else {
                    warn!(
                        "Unidad {} apunta a '{}', no al NAS esperado",
                        letter, current_unc
                    );
                    DriveValidation::PointsElsewhere {
                        current_target: current_unc,
                    }
                }
            } else {
                warn!(
                    "Unidad {} apunta a '{}', no al NAS esperado (fallback IP deshabilitado)",
                    letter, current_unc
                );
                DriveValidation::PointsElsewhere {
                    current_target: current_unc,
                }
            }
        }
        Err(e) => {
            error!("Error al consultar unidad {}: {}", letter, e);
            DriveValidation::Error(e.to_string())
        }
    }
}

/// Monta la unidad de red usando WNetAddConnection2W
#[cfg(windows)]
pub fn mount_drive_wnet(
    drive_letter: &str,
    unc_path: &str,
    username: Option<&str>,
    password: Option<&str>,
) -> Result<(), SincroniaError> {
    info!(
        "Montando {} → {} vía WNet API",
        drive_letter, unc_path
    );

    let mut local_wide: Vec<u16> = drive_letter.encode_utf16().chain(std::iter::once(0)).collect();
    let mut remote_wide: Vec<u16> = unc_path.encode_utf16().chain(std::iter::once(0)).collect();

    let user_wide: Option<Vec<u16>> = username.map(|u| u.encode_utf16().chain(std::iter::once(0)).collect());
    let pass_wide: Option<Vec<u16>> = password.map(|p| p.encode_utf16().chain(std::iter::once(0)).collect());

    let nr = NETRESOURCEW {
        dwScope: Default::default(),
        dwType: RESOURCETYPE_DISK,
        dwDisplayType: Default::default(),
        dwUsage: Default::default(),
        lpLocalName: windows::core::PWSTR::from_raw(local_wide.as_mut_ptr()),
        lpRemoteName: windows::core::PWSTR::from_raw(remote_wide.as_mut_ptr()),
        lpComment: windows::core::PWSTR::null(),
        lpProvider: windows::core::PWSTR::null(),
    };

    let password_pcwstr = pass_wide
        .as_ref()
        .map(|p| PCWSTR::from_raw(p.as_ptr()));
    let username_pcwstr = user_wide
        .as_ref()
        .map(|u| PCWSTR::from_raw(u.as_ptr()));

    // SAFETY: Llamamos a WNetAddConnection2W con estructura NETRESOURCEW válida.
    // Los punteros a strings anchos permanecen válidos durante toda la llamada.
    // No se persisten credenciales: se pasan solo para esta llamada.
    let result: WIN32_ERROR = unsafe {
        WNetAddConnection2W(
            &nr,
            password_pcwstr.unwrap_or(PCWSTR::null()),
            username_pcwstr.unwrap_or(PCWSTR::null()),
            NET_CONNECT_FLAGS(0), // No persistir la conexión
        )
    };

    if result.0 == NO_ERROR {
        info!("Unidad {} montada correctamente", drive_letter);
        Ok(())
    } else {
        error!(
            "WNetAddConnection2W falló (código: {})",
            result.0
        );
        Err(SincroniaError::Nas {
            message: format!(
                "No se pudo montar {} → {}: código de error {}",
                drive_letter, unc_path, result.0
            ),
        })
    }
}

#[cfg(not(windows))]
pub fn mount_drive_wnet(
    _drive_letter: &str,
    _unc_path: &str,
    _username: Option<&str>,
    _password: Option<&str>,
) -> Result<(), SincroniaError> {
    Err(SincroniaError::Nas {
        message: "WNet API solo disponible en Windows".into(),
    })
}

/// Desmonta la unidad de red usando WNetCancelConnection2W
#[cfg(windows)]
pub fn unmount_drive(drive_letter: &str) -> Result<(), SincroniaError> {
    warn!("Desmontando unidad {}", drive_letter);
    let local_wide: Vec<u16> = drive_letter.encode_utf16().chain(std::iter::once(0)).collect();

    // SAFETY: WNetCancelConnection2W desconecta la unidad de red.
    // El puntero local_wide permanece válido durante la llamada.
    // force=true fuerza la desconexión incluso si hay archivos abiertos.
    let result: WIN32_ERROR = unsafe {
        WNetCancelConnection2W(PCWSTR::from_raw(local_wide.as_ptr()), NET_CONNECT_FLAGS(0), true)
    };

    if result.0 == NO_ERROR {
        info!("Unidad {} desmontada correctamente", drive_letter);
        Ok(())
    } else {
        Err(SincroniaError::Nas {
            message: format!(
                "No se pudo desmontar {}: código de error {}",
                drive_letter, result.0
            ),
        })
    }
}

#[cfg(not(windows))]
pub fn unmount_drive(_drive_letter: &str) -> Result<(), SincroniaError> {
    Err(SincroniaError::Nas {
        message: "WNet API solo disponible en Windows".into(),
    })
}

/// Fallback: monta la unidad usando `net use` vía Command.
/// NOTA DE SEGURIDAD: net use puede exponer credenciales en la lista de
/// procesos del sistema. Se evita registrar la línea completa con credenciales.
pub fn mount_drive_net_use(
    drive_letter: &str,
    unc_path: &str,
    username: Option<&str>,
    password: Option<&str>,
) -> Result<(), SincroniaError> {
    warn!(
        "Usando fallback 'net use' para montar {} → {} (menos seguro que WNet API)",
        drive_letter, unc_path
    );

    let mut cmd = Command::new("net");
    cmd.arg("use").arg(drive_letter).arg(unc_path);

    if let Some(pass) = password {
        cmd.arg(pass);
    }
    if let Some(user) = username {
        cmd.arg(format!("/user:{}", user));
    }

    cmd.arg("/persistent:no");

    // Registrar comando SIN credenciales
    info!(
        "Ejecutando: net use {} {} /user:*** /persistent:no",
        drive_letter, unc_path
    );

    let output = cmd.output().map_err(|e| SincroniaError::Nas {
        message: format!("Error al ejecutar net use: {}", e),
    })?;

    if output.status.success() {
        info!("net use: unidad {} montada correctamente", drive_letter);
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        error!("net use falló: {}", stderr);
        Err(SincroniaError::Nas {
            message: format!("net use falló: {}", stderr.trim()),
        })
    }
}

/// Intenta montar la unidad probando las estrategias configuradas en orden
pub fn attempt_mount(
    config: &NasConfig,
    username: Option<&str>,
    password: Option<&str>,
) -> Result<(), SincroniaError> {
    let letter = &config.required_drive_letter;
    let primary = &config.primary_unc_path;

    if config.prefer_windows_wnet_api {
        match mount_drive_wnet(letter, primary, username, password) {
            Ok(()) => return Ok(()),
            Err(e) => warn!("WNet API falló con UNC primario: {}", e),
        }

        if config.allow_ip_fallback && !config.fallback_unc_path_by_ip.is_empty() {
            match mount_drive_wnet(letter, &config.fallback_unc_path_by_ip, username, password) {
                Ok(()) => return Ok(()),
                Err(e) => warn!("WNet API falló con IP fallback: {}", e),
            }
        }
    }

    if config.allow_net_use_fallback {
        match mount_drive_net_use(letter, primary, username, password) {
            Ok(()) => return Ok(()),
            Err(e) => warn!("net use falló con UNC primario: {}", e),
        }

        if config.allow_ip_fallback && !config.fallback_unc_path_by_ip.is_empty() {
            match mount_drive_net_use(letter, &config.fallback_unc_path_by_ip, username, password) {
                Ok(()) => return Ok(()),
                Err(e) => warn!("net use falló con IP fallback: {}", e),
            }
        }
    }

    Err(SincroniaError::Nas {
        message: format!(
            "No se pudo montar {} tras agotar todas las estrategias",
            letter
        ),
    })
}
