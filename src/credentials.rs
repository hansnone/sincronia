// sincronia/src/credentials.rs
//
// Prompt seguro de credenciales para sesiones interactivas.
// Nunca almacena contraseñas persistentemente.
// Detecta si hay sesión interactiva disponible.

use crate::errors::SincroniaError;
use tracing::{info, warn};

/// Credenciales temporales (la contraseña se descarta tras su uso)
pub struct Credentials {
    pub username: String,
    pub password: String,
}

impl Drop for Credentials {
    fn drop(&mut self) {
        // Sobrescribir la contraseña en memoria antes de liberar
        // Nota: esto no es una garantía criptográfica perfecta,
        // pero reduce la ventana de exposición en memoria.
        self.password = "0".repeat(self.password.len());
    }
}

/// Detecta si la sesión actual es interactiva (tiene consola)
#[cfg(windows)]
pub fn is_interactive_session() -> bool {
    use windows::Win32::System::Console::{GetConsoleMode, GetStdHandle, STD_INPUT_HANDLE};

    // SAFETY: GetStdHandle retorna un handle al dispositivo estándar de entrada.
    // Es una llamada de solo lectura, sin efectos secundarios.
    let handle = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
    match handle {
        Ok(h) => {
            let mut mode = Default::default();
            // SAFETY: GetConsoleMode consulta el modo de la consola.
            // Solo lectura, no modifica estado.
            unsafe { GetConsoleMode(h, &mut mode).is_ok() }
        }
        Err(_) => false,
    }
}

#[cfg(not(windows))]
pub fn is_interactive_session() -> bool {
    atty::is(atty::Stream::Stdin)
}

/// Solicita credenciales al usuario interactivamente.
/// - El usuario se introduce con stdin normal.
/// - La contraseña se introduce con rpassword (sin eco).
/// - Formatos de usuario aceptados: DOMINIO\usuario, usuario@dominio
/// - Retorna error si no hay sesión interactiva.
pub fn prompt_credentials(max_attempts: u32) -> Result<Credentials, SincroniaError> {
    if !is_interactive_session() {
        warn!("No hay sesión interactiva — no se pueden solicitar credenciales");
        return Err(SincroniaError::Credential {
            message: "No hay sesión interactiva disponible para solicitar credenciales. \
                      El programa esperará al próximo ciclo."
                .into(),
        });
    }

    for attempt in 1..=max_attempts {
        println!(
            "\n╔══════════════════════════════════════════════════════╗"
        );
        println!(
            "║  Sincronia — Credenciales de NAS (intento {}/{})     ║",
            attempt, max_attempts
        );
        println!(
            "╚══════════════════════════════════════════════════════╝"
        );
        println!("  Formatos aceptados: DOMINIO\\usuario  o  usuario@dominio");

        print!("  Usuario: ");
        // Flush stdout para que el prompt aparezca antes de leer
        use std::io::Write;
        std::io::stdout().flush().ok();

        let mut username = String::new();
        if std::io::stdin().read_line(&mut username).is_err() {
            warn!("Error leyendo nombre de usuario");
            continue;
        }
        let username = username.trim().to_string();

        if username.is_empty() {
            warn!("Usuario vacío — cancelando intento {}", attempt);
            println!("  ⚠ Usuario vacío. Intente de nuevo.");
            continue;
        }

        // Validar formato del usuario
        if !username.contains('\\') && !username.contains('@') {
            println!("  ⚠ Formato no reconocido. Use DOMINIO\\usuario o usuario@dominio.");
            warn!("Formato de usuario no válido: sin \\ ni @");
            continue;
        }

        match rpassword::read_password_from_tty(Some("  Contraseña: ")) {
            Ok(password) => {
                if password.is_empty() {
                    println!("  ⚠ Contraseña vacía. Intente de nuevo.");
                    warn!("Contraseña vacía en intento {}", attempt);
                    continue;
                }

                info!(
                    "Credenciales proporcionadas por usuario (intento {})",
                    attempt
                );
                return Ok(Credentials { username, password });
            }
            Err(e) => {
                warn!("Error leyendo contraseña: {}", e);
                println!("  ⚠ Error al leer contraseña: {}", e);
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
