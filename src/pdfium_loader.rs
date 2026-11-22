//! Lädt die PDFium-Bibliothek, die zur Laufzeit benötigt wird.
//!
//! Die Bibliothek wird beim Kompilieren heruntergeladen und in das Programm
//! eingebettet. Beim ersten Start wird sie in einen Cache-Ordner geschrieben
//! (macOS/Linux benötigen eine echte Datei, um die Bibliothek zu laden) und
//! von dort geladen.

use std::fs;
use std::path::PathBuf;

use pdfium_render::prelude::Pdfium;

// Erzeugt von build.rs: enthält die eingebettete Bibliothek.
include!(concat!(env!("OUT_DIR"), "/pdfium_bin.rs"));

/// Dateiname der Bibliothek auf der aktuellen Plattform.
pub const LIB_FILE_NAME: &str = if cfg!(target_os = "windows") {
    "pdfium.dll"
} else if cfg!(target_os = "macos") {
    "libpdfium.dylib"
} else {
    "libpdfium.so"
};

/// Ordner, in dem die Bibliothek zur Laufzeit abgelegt wird.
fn cache_dir() -> Result<PathBuf, String> {
    let base = if cfg!(target_os = "windows") {
        std::env::var("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."))
    } else {
        let home = std::env::var("HOME").map_err(|_| "HOME ist nicht gesetzt".to_string())?;
        PathBuf::from(home).join(".cache")
    };
    Ok(base.join("drawgrab").join("pdfium"))
}

/// Bindet PDFium und liefert eine einsatzbereite Instanz.
pub fn load_pdfium() -> Result<Pdfium, String> {
    let dir = cache_dir()?;
    let path = dir.join(LIB_FILE_NAME);

    let needs_write = match fs::metadata(&path) {
        Ok(meta) => meta.len() != PDFIUM_BYTES.len() as u64,
        Err(_) => true,
    };
    if needs_write {
        fs::create_dir_all(&dir)
            .map_err(|e| format!("Cache-Ordner {} nicht anlegbar: {e}", dir.display()))?;
        fs::write(&path, PDFIUM_BYTES)
            .map_err(|e| format!("PDFium konnte nicht geschrieben werden: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o755));
        }
    }

    Pdfium::bind_to_library(&path)
        .map(Pdfium::new)
        .map_err(|e| format!("PDFium konnte nicht geladen werden: {e}"))
}
