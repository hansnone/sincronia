// sincronia/src/exclusions.rs
//
// Filtrado de archivos y directorios según patrones configurados.
// Compila patrones glob una sola vez al crear el filtro.

use glob::Pattern;
use tracing::debug;

/// Filtro de exclusiones compilado
pub struct ExclusionFilter {
    /// Nombres de directorio a excluir (case-insensitive)
    excluded_dirs: Vec<String>,
    /// Patrones glob compilados para nombres de archivo
    excluded_patterns: Vec<Pattern>,
}

impl ExclusionFilter {
    /// Crea un nuevo filtro compilando los patrones glob
    pub fn new(dir_names: &[String], file_patterns: &[String]) -> Self {
        let excluded_dirs: Vec<String> = dir_names.iter().map(|d| d.to_lowercase()).collect();

        let excluded_patterns: Vec<Pattern> = file_patterns
            .iter()
            .filter_map(|p| match Pattern::new(p) {
                Ok(pattern) => Some(pattern),
                Err(e) => {
                    tracing::warn!("Patrón glob inválido '{}': {}", p, e);
                    None
                }
            })
            .collect();

        debug!(
            "Filtro de exclusiones: {} directorios, {} patrones",
            excluded_dirs.len(),
            excluded_patterns.len()
        );

        Self {
            excluded_dirs,
            excluded_patterns,
        }
    }

    /// Comprueba si un nombre de directorio debe ser excluido
    pub fn is_directory_excluded(&self, dir_name: &str) -> bool {
        let lower = dir_name.to_lowercase();
        self.excluded_dirs.iter().any(|d| d == &lower)
    }

    /// Comprueba si un nombre de archivo debe ser excluido
    pub fn is_file_excluded(&self, file_name: &str) -> bool {
        self.excluded_patterns
            .iter()
            .any(|p| p.matches(file_name) || p.matches(&file_name.to_lowercase()))
    }

    /// Comprueba si una ruta (archivo o directorio) debe ser excluida
    pub fn is_excluded(&self, name: &str, is_dir: bool) -> bool {
        if is_dir {
            self.is_directory_excluded(name)
        } else {
            self.is_file_excluded(name)
        }
    }
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_directory_exclusion() {
        let filter = ExclusionFilter::new(
            &[".stfolder".into(), "$RECYCLE.BIN".into()],
            &[],
        );
        assert!(filter.is_directory_excluded(".stfolder"));
        assert!(filter.is_directory_excluded(".STFOLDER")); // case-insensitive
        assert!(filter.is_directory_excluded("$RECYCLE.BIN"));
        assert!(!filter.is_directory_excluded("Documents"));
    }

    #[test]
    fn test_file_pattern_exclusion() {
        let filter = ExclusionFilter::new(
            &[],
            &[
                "*.tmp".into(),
                "~*.*".into(),
                "Thumbs.db".into(),
                ".DS_Store".into(),
                "*.partial".into(),
            ],
        );

        assert!(filter.is_file_excluded("archivo.tmp"));
        assert!(filter.is_file_excluded("~documento.docx"));
        assert!(filter.is_file_excluded("Thumbs.db"));
        assert!(filter.is_file_excluded(".DS_Store"));
        assert!(filter.is_file_excluded("video.partial"));
        assert!(!filter.is_file_excluded("video.mp4"));
        assert!(!filter.is_file_excluded("documento.pdf"));
    }

    #[test]
    fn test_is_excluded_combined() {
        let filter = ExclusionFilter::new(
            &["$RECYCLE.BIN".into()],
            &["*.tmp".into()],
        );

        assert!(filter.is_excluded("$RECYCLE.BIN", true));
        assert!(filter.is_excluded("test.tmp", false));
        assert!(!filter.is_excluded("test.txt", false));
        assert!(!filter.is_excluded("Documents", true));
    }

    #[test]
    fn test_invalid_pattern_ignored() {
        // Un patrón inválido no debe causar pánico
        let filter = ExclusionFilter::new(&[], &["[invalid".into(), "*.tmp".into()]);
        // El patrón válido sigue funcionando
        assert!(filter.is_file_excluded("test.tmp"));
    }
}
