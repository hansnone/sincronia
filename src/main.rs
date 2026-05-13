// sincronia/src/main.rs
//
// Punto de entrada principal de Sincronia.
// Parsea argumentos, carga configuración, lanza orquestador y tray.
// El tray DEBE ejecutarse en el thread principal (requisito Windows).

#![cfg_attr(
    not(debug_assertions),
    windows_subsystem = "windows"
)]

mod config;
mod conflict;
mod copy_engine;
mod errors;
mod exclusions;
mod hasher;
mod logging;
mod metadata;
mod orchestrator;
mod planner;
mod scanner;
mod scheduled_task;
mod scheduler;
mod shutdown;
mod stability;
mod stats;
mod tray;
mod verifier;

use crate::config::SincroniaConfig;
use crate::orchestrator::{Orchestrator, OrchestratorMessage, TrayCommand};
use crate::shutdown::ShutdownSignal;
use crate::tray::TrayConfig;
use std::path::{Path, PathBuf};

fn main() {
    // Parsear argumentos
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--version" || a == "-v") {
        println!("Sincronia v{}", env!("CARGO_PKG_VERSION"));
        println!("Motor de sincronización multipar de alto rendimiento para Windows 11");
        return;
    }

    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return;
    }

    if args.iter().any(|a| a == "--generate-config") {
        let output = args
            .iter()
            .position(|a| a == "--generate-config")
            .and_then(|i| args.get(i + 1))
            .map(|s| s.as_str())
            .unwrap_or("sincronia.toml");

        match std::fs::write(output, SincroniaConfig::generate_example_toml()) {
            Ok(()) => {
                println!("Archivo de configuración de ejemplo generado: {}", output);
                println!("Edite los valores y ejecute: sincronia --config {}", output);
            }
            Err(e) => {
                eprintln!("Error al escribir archivo de configuración: {}", e);
                std::process::exit(1);
            }
        }
        return;
    }

    // Determinar ruta del archivo de configuración
    let config_path = args
        .iter()
        .position(|a| a == "--config")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            // Buscar en el directorio del ejecutable
            let exe_dir = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|d| d.to_path_buf()))
                .unwrap_or_else(|| PathBuf::from("."));
            exe_dir.join("sincronia.toml")
        });

    if !config_path.exists() {
        eprintln!(
            "Error: No se encontró el archivo de configuración: {}",
            config_path.display()
        );
        eprintln!("Use --generate-config para crear uno de ejemplo.");
        eprintln!("Use --config <ruta> para especificar la ubicación.");
        std::process::exit(1);
    }

    // Cargar configuración
    let config = match SincroniaConfig::load_from_file(&config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error en la configuración: {}", e);
            std::process::exit(1);
        }
    };

    // Crear tarea programada si se solicita
    if args.iter().any(|a| a == "--create-scheduled-task") {
        let exe_path = std::env::current_exe()
            .unwrap_or_else(|_| PathBuf::from("sincronia.exe"));

        match scheduled_task::create_scheduled_task(
            &config.scheduled_task,
            &exe_path.to_string_lossy(),
            &config_path.to_string_lossy(),
        ) {
            Ok(()) => println!("Tarea programada creada correctamente."),
            Err(e) => {
                eprintln!("Error al crear tarea programada: {}", e);
                std::process::exit(1);
            }
        }
        return;
    }

    // Modo sin tray (para depuración o ejecución en background)
    let no_tray = args.iter().any(|a| a == "--no-tray");

    // ── Iniciar sistema ──
    println!("═══════════════════════════════════════════════════");
    println!("  Sincronia v{}", env!("CARGO_PKG_VERSION"));
    println!("  Motor de sincronización multipar");
    println!("  Config: {}", config_path.display());
    for (i, pair) in config.sync_pairs.iter().enumerate() {
        println!(
            "  Par {}: {} [{}] → {} [{}]",
            i + 1,
            pair.source_path.display(),
            pair.source_virtual_drive_letter,
            pair.target_path.display(),
            pair.target_virtual_drive_letter
        );
    }
    println!("  Workers: {} (máx: {})", config.copy_engine.worker_count, config.copy_engine.maximum_worker_count);
    println!("  Buffer: {} MiB/worker", config.copy_engine.copy_buffer_size_mib_per_worker);
    println!("═══════════════════════════════════════════════════");

    let shutdown = ShutdownSignal::new();
    shutdown.register_ctrlc_handler();

    if no_tray {
        // Modo sin tray — ejecutar orquestador en thread principal
        let mut orchestrator = Orchestrator::new(config, shutdown, None, None);
        orchestrator.run();
    } else {
        // Modo normal — tray en thread principal, orquestador en thread secundario
        let (orch_tx, orch_rx) = crossbeam_channel::unbounded::<OrchestratorMessage>();
        let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded::<TrayCommand>();

        let config_clone = config.clone();
        let shutdown_clone = shutdown.clone();

        // Lanzar orquestador en thread secundario
        let orch_handle = std::thread::Builder::new()
            .name("sincronia-orchestrator".to_string())
            .spawn(move || {
                let mut orchestrator = Orchestrator::new(
                    config_clone,
                    shutdown_clone,
                    Some(orch_tx),
                    Some(cmd_rx),
                );
                orchestrator.run();
            })
            .expect("Error al crear thread del orquestador");

        // Tray en thread principal (requisito Windows)
        let log_dir = config
            .logging
            .human_log_file_path
            .parent()
            .unwrap_or(Path::new("."))
            .to_path_buf();

        let sync_pair_menu_lines: Vec<String> = config
            .sync_pairs
            .iter()
            .enumerate()
            .map(|(i, p)| {
                format!(
                    "Par {} — {} [{}] → {} [{}]",
                    i + 1,
                    p.source_path.display(),
                    p.source_virtual_drive_letter,
                    p.target_path.display(),
                    p.target_virtual_drive_letter
                )
            })
            .collect();

        let tray_config = TrayConfig {
            application_name: config.general.application_name.clone(),
            log_directory: log_dir,
            config_file_path: config_path.clone(),
            metrics_csv_path: config.logging.metrics_csv_file_path.clone(),
            sync_pair_menu_lines,
        };

        let global_state = std::sync::Arc::new(parking_lot::RwLock::new(
            errors::GlobalState::Starting,
        ));

        tray::run_tray(tray_config, orch_rx, cmd_tx, global_state);

        // Si el tray termina, esperar al orquestador
        shutdown.trigger();
        orch_handle.join().ok();
    }

    println!("Sincronia finalizado.");
}

fn print_help() {
    println!(
        r#"Sincronia v{version} — Sincronización multipar (Windows)

USO:
    sincronia [OPCIONES]

OPCIONES:
    --config <ruta>           Ruta al archivo de configuración TOML
                              (por defecto: sincronia.toml junto al ejecutable)

    --generate-config [ruta]  Genera un archivo de configuración de ejemplo
                              (por defecto: sincronia.toml)

    --create-scheduled-task   Crea una tarea programada de Windows según
                              la configuración del archivo TOML

    --no-tray                 Ejecutar sin icono en bandeja del sistema
                              (útil para depuración o ejecución como servicio)

    --version, -v             Muestra la versión del programa

    --help, -h                Muestra esta ayuda

EJEMPLOS:
    sincronia --generate-config sincronia.toml
    sincronia --config C:\Config\sincronia.toml
    sincronia --create-scheduled-task --config sincronia.toml
    sincronia --no-tray --config sincronia.toml

NOTAS:
    - El programa NO requiere privilegios de administrador.
    - Configure [[sync_pairs]] con rutas reales y letras virtuales (DefineDosDeviceW).
    - Para crear la tarea programada, ejecute una vez con --create-scheduled-task.
    - Los logs se generan en la ruta configurada en el archivo TOML.
"#,
        version = env!("CARGO_PKG_VERSION")
    );
}
