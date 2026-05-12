// sincronia/src/credentials.rs
//
// Diálogo nativo de credenciales Windows (CredUI).
// Usa CredUIPromptForWindowsCredentialsW para mostrar el diálogo estándar del sistema.
// Funciona sin consola — perfecto para tareas programadas al inicio de sesión.
// Las credenciales NUNCA se almacenan en disco.

use crate::errors::SincroniaError;
use tracing::{info, warn};

/// Credenciales temporales (la contraseña se sobrescribe en memoria al hacer Drop)
pub struct Credentials {
    pub username: String,
    pub password: String,
}

impl Drop for Credentials {
    fn drop(&mut self) {
        // Sobrescribir la contraseña en memoria antes de liberar.
        // Nota: esto no es una garantía criptográfica perfecta,
        // pero reduce la ventana de exposición en memoria.
        self.password = "0".repeat(self.password.len());
    }
}

/// Código de error: el usuario canceló el diálogo
#[cfg(windows)]
const ERROR_CANCELLED: u32 = 1223;

/// Solicita credenciales al usuario mediante el diálogo nativo de Windows.
///
/// Muestra el diálogo estándar de CredUI (el mismo que aparece al mapear
/// una unidad de red manualmente). Funciona sin consola adjunta.
///
/// # Argumentos
/// - `unc_path`: ruta UNC del NAS (se muestra en el mensaje del diálogo)
/// - `max_attempts`: número máximo de intentos antes de fallar
///
/// # Errores
/// - `SincroniaError::Cancelled` si el usuario cierra el diálogo
/// - `SincroniaError::Credential` si falla la API o se agotan los intentos
#[cfg(windows)]
pub fn prompt_credentials_gui(
    unc_path: &str,
    max_attempts: u32,
) -> Result<Credentials, SincroniaError> {
    use std::ffi::c_void;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Gdi::HBITMAP;
    use windows::Win32::Security::Credentials::{
        CredUIPromptForWindowsCredentialsW,
        CREDUI_INFOW, CREDUIWIN_GENERIC,
    };
    use windows::Win32::System::Com::CoTaskMemFree;

    // Strings anchos para el diálogo
    let caption = "Sincronia — Credenciales NAS";
    let message = format!(
        "Ingrese sus credenciales para conectar al recurso de red:\n{}",
        unc_path
    );

    let caption_wide: Vec<u16> = caption.encode_utf16().chain(std::iter::once(0)).collect();
    let message_wide: Vec<u16> = message.encode_utf16().chain(std::iter::once(0)).collect();

    for attempt in 1..=max_attempts {
        info!(
            "Mostrando diálogo de credenciales (intento {}/{})",
            attempt, max_attempts
        );

        let ui_info = CREDUI_INFOW {
            cbSize: std::mem::size_of::<CREDUI_INFOW>() as u32,
            hwndParent: HWND::default(),
            pszMessageText: PCWSTR::from_raw(message_wide.as_ptr()),
            pszCaptionText: PCWSTR::from_raw(caption_wide.as_ptr()),
            hbmBanner: HBITMAP::default(),
        };

        let mut auth_package: u32 = 0;
        let mut out_buffer: *mut c_void = std::ptr::null_mut();
        let mut out_buffer_size: u32 = 0;
        // SAFETY: CredUIPromptForWindowsCredentialsW muestra un diálogo modal del sistema.
        // Los punteros a strings anchos (caption_wide, message_wide) permanecen válidos
        // durante toda la llamada porque están en el scope del for loop.
        // out_buffer es asignado por el sistema y debe liberarse con CoTaskMemFree.
        // pfsave = None: no mostrar checkbox "Recordar credenciales".
        let result = unsafe {
            CredUIPromptForWindowsCredentialsW(
                Some(&ui_info as *const CREDUI_INFOW),
                0, // dwAuthError: 0 = primera solicitud
                &mut auth_package,
                None,    // pInAuthBuffer: sin buffer previo
                0,       // ulInAuthBufferSize
                &mut out_buffer,
                &mut out_buffer_size,
                None,    // pfsave: sin checkbox "Recordar"
                CREDUIWIN_GENERIC, // Credenciales genéricas (usuario + contraseña)
            )
        };

        if result == ERROR_CANCELLED {
            info!("Usuario canceló el diálogo de credenciales");
            return Err(SincroniaError::Cancelled);
        }

        if result != 0 {
            warn!(
                "CredUIPromptForWindowsCredentialsW falló con código: {}",
                result
            );
            // Liberar buffer si se asignó
            if !out_buffer.is_null() {
                // SAFETY: out_buffer fue asignado por CredUIPromptForWindowsCredentialsW.
                // CoTaskMemFree acepta punteros nulos sin efecto.
                unsafe { CoTaskMemFree(Some(out_buffer)) };
            }
            continue;
        }

        // Desempaquetar usuario, dominio y contraseña del buffer de autenticación
        let creds = unpack_auth_buffer(out_buffer, out_buffer_size);

        // SAFETY: out_buffer fue asignado por CredUIPromptForWindowsCredentialsW.
        // Debemos liberarlo con CoTaskMemFree independientemente del resultado del unpack.
        unsafe { CoTaskMemFree(Some(out_buffer)) };

        match creds {
            Ok(c) => {
                if c.username.is_empty() {
                    warn!("Usuario vacío en intento {}", attempt);
                    continue;
                }
                if c.password.is_empty() {
                    warn!("Contraseña vacía en intento {}", attempt);
                    continue;
                }
                info!(
                    "Credenciales proporcionadas por el usuario (intento {})",
                    attempt
                );
                return Ok(c);
            }
            Err(e) => {
                warn!("Error al desempaquetar credenciales: {}", e);
                continue;
            }
        }
    }

    Err(SincroniaError::Credential {
        message: format!(
            "Se agotaron los {} intentos de credenciales",
            max_attempts
        ),
    })
}

/// Desempaqueta el buffer de autenticación devuelto por CredUIPromptForWindowsCredentialsW.
///
/// Extrae usuario (con dominio si corresponde) y contraseña.
/// El formato del usuario resultante será "DOMINIO\usuario" si se proporcionó dominio.
#[cfg(windows)]
fn unpack_auth_buffer(
    auth_buffer: *mut std::ffi::c_void,
    auth_buffer_size: u32,
) -> Result<Credentials, SincroniaError> {
    use windows::core::PWSTR;
    use windows::Win32::Security::Credentials::{
        CredUnPackAuthenticationBufferW, CRED_PACK_FLAGS,
    };

    // Buffers para usuario, dominio y contraseña (CREDUI_MAX_USERNAME_LENGTH = 513)
    const MAX_LEN: usize = 513;
    let mut user_buf = [0u16; MAX_LEN];
    let mut domain_buf = [0u16; MAX_LEN];
    let mut pass_buf = [0u16; MAX_LEN];
    let mut user_size = MAX_LEN as u32;
    let mut domain_size = MAX_LEN as u32;
    let mut pass_size = MAX_LEN as u32;

    // SAFETY: CredUnPackAuthenticationBufferW extrae credenciales del buffer de autenticación.
    // auth_buffer es válido y fue asignado por CredUIPromptForWindowsCredentialsW.
    // Los buffers de salida tienen tamaño suficiente (513 caracteres cada uno).
    // Las variables de tamaño se actualizan con la longitud real de cada campo.
    unsafe {
        CredUnPackAuthenticationBufferW(
            CRED_PACK_FLAGS(0), // Sin flags especiales
            auth_buffer as *const std::ffi::c_void,
            auth_buffer_size,
            Some(PWSTR::from_raw(user_buf.as_mut_ptr())),
            &mut user_size,
            Some(PWSTR::from_raw(domain_buf.as_mut_ptr())),
            Some(&mut domain_size),
            Some(PWSTR::from_raw(pass_buf.as_mut_ptr())),
            &mut pass_size,
        )
        .map_err(|e| SincroniaError::Credential {
            message: format!("CredUnPackAuthenticationBufferW falló: {}", e),
        })?;
    }

    // Convertir buffers UTF-16 a String
    let username = String::from_utf16_lossy(&user_buf[..user_size as usize])
        .trim_end_matches('\0')
        .to_string();
    let domain = String::from_utf16_lossy(&domain_buf[..domain_size as usize])
        .trim_end_matches('\0')
        .to_string();
    let password = String::from_utf16_lossy(&pass_buf[..pass_size as usize])
        .trim_end_matches('\0')
        .to_string();

    // Sobrescribir buffer de contraseña en memoria
    pass_buf.fill(0);

    // Construir usuario con dominio si corresponde
    // Si el usuario ya incluye dominio (DOMINIO\user o user@dominio), usarlo tal cual.
    // Si hay dominio separado, prefijarlo.
    let full_username = if !domain.is_empty()
        && !username.contains('\\')
        && !username.contains('@')
    {
        format!("{}\\{}", domain, username)
    } else {
        username
    };

    Ok(Credentials {
        username: full_username,
        password,
    })
}

/// Versión no-Windows: siempre falla (CredUI no disponible)
#[cfg(not(windows))]
pub fn prompt_credentials_gui(
    _unc_path: &str,
    _max_attempts: u32,
) -> Result<Credentials, SincroniaError> {
    Err(SincroniaError::Credential {
        message: "Diálogo de credenciales solo disponible en Windows".into(),
    })
}
