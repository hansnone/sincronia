// sincronia/src/tray.rs
//
// Icono en bandeja del sistema con menú contextual.
// Usa tray-icon + muda para menú + winit para event loop.
// Tres estados de color: verde, amarillo, rojo.

use crate::errors::{GlobalState, TrayColor};
use crate::orchestrator::{OrchestratorMessage, TrayCommand};
use crossbeam_channel::{Receiver, Sender};
use parking_lot::RwLock;
use std::sync::Arc;
use tracing::{debug, error, info};

#[cfg(windows)]
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
#[cfg(windows)]
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
#[cfg(windows)]
use winit::application::ApplicationHandler;
#[cfg(windows)]
use winit::event::WindowEvent;
#[cfg(windows)]
use winit::event_loop::{ActiveEventLoop, EventLoop};
#[cfg(windows)]
use winit::window::WindowId;

/// IDs de menú
const MENU_STATUS: &str = "status";
const MENU_PAUSE: &str = "pause";
const MENU_RESUME: &str = "resume";
const MENU_STOP: &str = "stop";
const MENU_OPEN_LOGS: &str = "open_logs";
const MENU_OPEN_CONFIG: &str = "open_config";
const MENU_OPEN_METRICS: &str = "open_metrics";
const MENU_QUIT: &str = "quit";

/// Eventos personalizados para winit
#[derive(Debug, Clone)]
pub enum TrayEvent {
    OrchestratorMessage(OrchestratorMessage),
    MenuAction(String),
}

/// Configuración del tray
pub struct TrayConfig {
    pub application_name: String,
    pub log_directory: std::path::PathBuf,
    pub config_file_path: std::path::PathBuf,
    pub metrics_csv_path: std::path::PathBuf,
    /// Líneas informativas por par (menú deshabilitado)
    pub sync_pair_menu_lines: Vec<String>,
}

/// Ejecuta el loop del tray icon (DEBE ejecutarse en el thread principal)
#[cfg(windows)]
pub fn run_tray(
    tray_config: TrayConfig,
    orchestrator_receiver: Receiver<OrchestratorMessage>,
    command_sender: Sender<TrayCommand>,
    global_state: Arc<RwLock<GlobalState>>,
) {
    info!("Iniciando sistema de bandeja...");

    let event_loop = EventLoop::<TrayEvent>::with_user_event()
        .build()
        .expect("Error al crear event loop");

    let proxy = event_loop.create_proxy();

    // Configurar handler para eventos de menú
    let proxy_menu = proxy.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let _ = proxy_menu.send_event(TrayEvent::MenuAction(event.id().0.clone()));
    }));

    // Forward de mensajes del orquestador al event loop
    let proxy_orch = proxy.clone();
    std::thread::spawn(move || {
        while let Ok(msg) = orchestrator_receiver.recv() {
            if proxy_orch
                .send_event(TrayEvent::OrchestratorMessage(msg))
                .is_err()
            {
                break;
            }
        }
    });

    let mut app = TrayApp {
        tray_icon: None,
        tray_config,
        command_sender,
        global_state,
        menu_status_item: None,
        _hidden_window: None,
    };

    event_loop.run_app(&mut app).ok();
}

#[cfg(windows)]
struct TrayApp {
    tray_icon: Option<TrayIcon>,
    tray_config: TrayConfig,
    command_sender: Sender<TrayCommand>,
    global_state: Arc<RwLock<GlobalState>>,
    menu_status_item: Option<MenuItem>,
    _hidden_window: Option<winit::window::Window>,
}

#[cfg(windows)]
impl ApplicationHandler<TrayEvent> for TrayApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.tray_icon.is_some() {
            return; // Ya inicializado
        }

        // Crear una ventana oculta para evitar que el event loop de winit 0.30
        // salga automáticamente por no tener ventanas abiertas.
        if self._hidden_window.is_none() {
            let attrs = winit::window::Window::default_attributes()
                .with_visible(false)
                .with_title("Sincronia Hidden EventLoop Keeper");
            if let Ok(window) = event_loop.create_window(attrs) {
                self._hidden_window = Some(window);
            }
        }

        // Crear menú contextual
        let menu = Menu::new();

        let status_item = MenuItem::with_id(
            MENU_STATUS,
            "Estado: Iniciando...",
            false, // disabled — solo informativo
            None,
        );
        menu.append(&status_item).ok();
        menu.append(&PredefinedMenuItem::separator()).ok();

        for line in &self.tray_config.sync_pair_menu_lines {
            menu.append(&MenuItem::new(line, false, None))
                .ok();
        }
        menu.append(&PredefinedMenuItem::separator()).ok();

        menu.append(&MenuItem::with_id(MENU_PAUSE, "Pausar", true, None)).ok();
        menu.append(&MenuItem::with_id(MENU_RESUME, "Reanudar", true, None)).ok();
        menu.append(&MenuItem::with_id(MENU_STOP, "Parar ordenadamente", true, None)).ok();
        menu.append(&PredefinedMenuItem::separator()).ok();
        menu.append(&MenuItem::with_id(MENU_OPEN_LOGS, "Abrir carpeta de logs", true, None)).ok();
        menu.append(&MenuItem::with_id(MENU_OPEN_CONFIG, "Abrir configuración", true, None)).ok();
        menu.append(&MenuItem::with_id(MENU_OPEN_METRICS, "Abrir métricas", true, None)).ok();
        menu.append(&PredefinedMenuItem::separator()).ok();
        menu.append(&MenuItem::with_id(MENU_QUIT, "Salir", true, None)).ok();

        self.menu_status_item = Some(status_item);

        // Crear icono por defecto (amarillo — iniciando)
        let icon = create_colored_icon(TrayColor::Yellow);

        match TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip(&self.tray_config.application_name)
            .with_icon(icon)
            .build()
        {
            Ok(tray) => {
                info!("Icono de bandeja creado correctamente");
                self.tray_icon = Some(tray);
            }
            Err(e) => {
                error!("Error al crear icono de bandeja: {}", e);
            }
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: TrayEvent) {
        match event {
            TrayEvent::OrchestratorMessage(msg) => match msg {
                OrchestratorMessage::StateChanged(state) => {
                    let color = state.tray_color();
                    let full_label = format!("Estado: {}", state);
                    let label = truncate_menu_text(&full_label, 96);

                    if let Some(ref item) = self.menu_status_item {
                        item.set_text(&label);
                    }

                    if let Some(ref tray) = self.tray_icon {
                        let icon = create_colored_icon(color);
                        tray.set_icon(Some(icon)).ok();
                        let tip = format!("{} — {}", self.tray_config.application_name, state);
                        let tip = truncate_menu_text(&tip, 120);
                        tray.set_tooltip(Some(&tip)).ok();
                    }
                }
                OrchestratorMessage::Notification { title, message } => {
                    info!("Notificación: {} — {}", title, message);
                    // Balloon notifications handled via tray_icon tooltip for now
                    // Full balloon support requires Shell_NotifyIconW with NIF_INFO
                }
                OrchestratorMessage::CycleCompleted {
                    files_copied,
                    bytes_copied,
                } => {
                    let mb = bytes_copied as f64 / 1_048_576.0;
                    debug!(
                        "Ciclo completado: {} archivos, {:.1} MB",
                        files_copied, mb
                    );
                }
            },
            TrayEvent::MenuAction(id) => {
                debug!("Acción de menú: {}", id);
                match id.as_str() {
                    MENU_PAUSE => {
                        self.command_sender.send(TrayCommand::Pause).ok();
                    }
                    MENU_RESUME => {
                        self.command_sender.send(TrayCommand::Resume).ok();
                    }
                    MENU_STOP => {
                        self.command_sender.send(TrayCommand::Stop).ok();
                    }
                    MENU_OPEN_LOGS => {
                        open::that(&self.tray_config.log_directory).ok();
                    }
                    MENU_OPEN_CONFIG => {
                        open::that(&self.tray_config.config_file_path).ok();
                    }
                    MENU_OPEN_METRICS => {
                        open::that(&self.tray_config.metrics_csv_path).ok();
                    }
                    MENU_QUIT => {
                        self.command_sender.send(TrayCommand::Stop).ok();
                        event_loop.exit();
                    }
                    _ => {}
                }
            }
        }
    }

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        _event: WindowEvent,
    ) {
        // No hay ventana — solo tray
    }
}

#[cfg(windows)]
fn truncate_menu_text(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    format!("{}…", truncated)
}

/// Crea un icono de color sólido 16x16 RGBA para la bandeja
#[cfg(windows)]
fn create_colored_icon(color: TrayColor) -> Icon {
    let (r, g, b) = match color {
        TrayColor::Green => (46u8, 204, 113),  // Esmeralda
        TrayColor::Yellow => (241, 196, 15),    // Oro
        TrayColor::Red => (231, 76, 60),        // Carmesí
    };

    let size = 16;
    let mut rgba = Vec::with_capacity(size * size * 4);

    for y in 0..size {
        for x in 0..size {
            // Crear un círculo con borde suave
            let cx = (x as f32) - (size as f32 / 2.0) + 0.5;
            let cy = (y as f32) - (size as f32 / 2.0) + 0.5;
            let dist = (cx * cx + cy * cy).sqrt();
            let radius = size as f32 / 2.0 - 1.0;

            if dist < radius - 0.5 {
                rgba.extend_from_slice(&[r, g, b, 255]);
            } else if dist < radius + 0.5 {
                // Anti-aliasing en el borde
                let alpha = ((radius + 0.5 - dist) * 255.0) as u8;
                rgba.extend_from_slice(&[r, g, b, alpha]);
            } else {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }

    Icon::from_rgba(rgba, size as u32, size as u32).expect("Error creando icono")
}

/// Versión no-Windows (stub)
#[cfg(not(windows))]
pub fn run_tray(
    _tray_config: TrayConfig,
    _orchestrator_receiver: Receiver<OrchestratorMessage>,
    _command_sender: Sender<TrayCommand>,
    _global_state: Arc<RwLock<GlobalState>>,
) {
    info!("Tray icon no disponible en esta plataforma");
}
