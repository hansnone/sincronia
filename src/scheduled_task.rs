// sincronia/src/scheduled_task.rs
//
// Creación de tarea programada de Windows mediante PowerShell.
// Configura la tarea para ejecutar Sincronia al iniciar sesión del usuario.

use crate::config::ScheduledTaskConfig;
use std::process::Command;
use tracing::{error, info, warn};

/// Crea una tarea programada en Windows usando PowerShell
pub fn create_scheduled_task(
    config: &ScheduledTaskConfig,
    executable_path: &str,
    config_file_path: &str,
) -> Result<(), String> {
    if !config.create_scheduled_task {
        info!("Creación de tarea programada deshabilitada en configuración");
        return Ok(());
    }

    info!(
        "Creando tarea programada: '{}'",
        config.scheduled_task_name
    );

    let run_level = if config.run_with_highest_privileges {
        "Highest"
    } else {
        "Limited"
    };

    let logon_type = if config.run_only_when_user_is_logged_on {
        "Interactive"
    } else {
        "Password"
    };

    // Nota: si run_only_when_user_is_logged_on es false, el programa
    // no podrá solicitar credenciales interactivamente.
    if !config.run_only_when_user_is_logged_on {
        warn!(
            "La tarea se ejecutará sin sesión interactiva. \
             No se podrán solicitar credenciales al usuario."
        );
    }

    let delay = format!("PT{}S", config.delay_after_logon_seconds);

    // Construir script PowerShell
    let ps_script = format!(
        r#"
$ErrorActionPreference = 'Stop'

$taskName = '{task_name}'
$exePath = '{exe_path}'
$configPath = '{config_path}'

# Eliminar tarea anterior si existe
$existing = Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
if ($existing) {{
    Unregister-ScheduledTask -TaskName $taskName -Confirm:$false
    Write-Host "Tarea anterior eliminada."
}}

# Crear trigger: al iniciar sesión del usuario
$trigger = New-ScheduledTaskTrigger -AtLogOn
$trigger.Delay = '{delay}'

# Crear acción
$action = New-ScheduledTaskAction -Execute $exePath -Argument "--config `"$configPath`""

# Configurar settings
$settings = New-ScheduledTaskSettingsSet `
    -AllowStartIfOnBatteries `
    -DontStopIfGoingOnBatteries `
    -StartWhenAvailable `
    -MultipleInstances IgnoreNew `
    -RestartCount 3 `
    -RestartInterval (New-TimeSpan -Minutes 5) `
    -ExecutionTimeLimit (New-TimeSpan -Days 0)

# Crear principal
$principal = New-ScheduledTaskPrincipal `
    -UserId $env:USERNAME `
    -LogonType {logon_type} `
    -RunLevel {run_level}

# Registrar tarea
Register-ScheduledTask `
    -TaskName $taskName `
    -Trigger $trigger `
    -Action $action `
    -Settings $settings `
    -Principal $principal `
    -Description "Sincronia - Motor de respaldo NAS de alto rendimiento"

Write-Host "Tarea '$taskName' creada correctamente."
"#,
        task_name = config.scheduled_task_name.replace('\'', "''"),
        exe_path = executable_path.replace('\'', "''"),
        config_path = config_file_path.replace('\'', "''"),
        delay = delay,
        logon_type = logon_type,
        run_level = run_level,
    );

    // Ejecutar PowerShell
    let output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &ps_script,
        ])
        .output()
        .map_err(|e| format!("Error al ejecutar PowerShell: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if !stdout.is_empty() {
        info!("PowerShell output: {}", stdout.trim());
    }

    if output.status.success() {
        info!(
            "Tarea programada '{}' creada correctamente",
            config.scheduled_task_name
        );
        Ok(())
    } else {
        error!(
            "Error al crear tarea programada: {}",
            stderr.trim()
        );
        Err(format!(
            "PowerShell terminó con error: {}",
            stderr.trim()
        ))
    }
}

/// Elimina una tarea programada existente
pub fn remove_scheduled_task(task_name: &str) -> Result<(), String> {
    let ps_script = format!(
        r#"
$ErrorActionPreference = 'Stop'
$existing = Get-ScheduledTask -TaskName '{}' -ErrorAction SilentlyContinue
if ($existing) {{
    Unregister-ScheduledTask -TaskName '{}' -Confirm:$false
    Write-Host "Tarea eliminada."
}} else {{
    Write-Host "La tarea no existe."
}}
"#,
        task_name.replace('\'', "''"),
        task_name.replace('\'', "''"),
    );

    let output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &ps_script,
        ])
        .output()
        .map_err(|e| format!("Error al ejecutar PowerShell: {}", e))?;

    if output.status.success() {
        info!("Tarea '{}' eliminada", task_name);
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("Error al eliminar tarea: {}", stderr.trim()))
    }
}
