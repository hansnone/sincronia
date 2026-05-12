// sincronia/src/shutdown.rs
//
// Gestión de parada ordenada.
// Captura Ctrl+C y señales del menú de bandeja.
// Workers verifican la señal entre archivos.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing::info;

/// Señal de parada compartida entre todos los componentes
#[derive(Clone)]
pub struct ShutdownSignal {
    flag: Arc<AtomicBool>,
}

impl ShutdownSignal {
    /// Crea una nueva señal de parada
    pub fn new() -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Registra el handler de Ctrl+C
    pub fn register_ctrlc_handler(&self) {
        let flag = self.flag.clone();
        ctrlc::set_handler(move || {
            info!("Ctrl+C recibido — iniciando parada ordenada...");
            flag.store(true, Ordering::SeqCst);
        })
        .expect("Error al registrar handler de Ctrl+C");
    }

    /// Activa la señal de parada (desde menú tray u otra fuente)
    pub fn trigger(&self) {
        info!("Señal de parada activada");
        self.flag.store(true, Ordering::SeqCst);
    }

    /// Comprueba si se ha solicitado la parada
    pub fn is_shutdown_requested(&self) -> bool {
        self.flag.load(Ordering::Relaxed)
    }

    /// Obtiene referencia al AtomicBool para pasar a workers
    pub fn as_atomic(&self) -> Arc<AtomicBool> {
        self.flag.clone()
    }
}

impl Default for ShutdownSignal {
    fn default() -> Self {
        Self::new()
    }
}
