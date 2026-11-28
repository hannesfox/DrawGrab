//! Lädt die PDFium-Bibliothek, die zur Laufzeit benötigt wird.
//!
//! Die Bibliothek wird beim Kompilieren heruntergeladen und in das Programm
//! eingebettet. Beim ersten Start wird sie in einen Cache-Ordner geschrieben
//! (macOS/Linux benötigen eine echte Datei, um die Bibliothek zu laden) und
//! von dort geladen.

use std::fs;
use std::path::PathBuf;

use pdfium_render::prelude::{
    PdfFontCharacterSet, Pdfium, PdfiumCustomFontProvider, PdfiumCustomFontProviderRequest,
    PdfiumCustomFontProviderResponse,
};

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

    let mut pdfium = Pdfium::bind_to_library(&path)
        .map(Pdfium::new)
        .map_err(|e| format!("PDFium konnte nicht geladen werden: {e}"))?;
    // Ersatzschrift für PDFs ohne eingebettete Fonts (siehe unten).
    pdfium.set_custom_font_provider(Box::new(SystemFontFallback::new()));
    Ok(pdfium)
}

/// Kategorie der angefragten Schrift (aus den PDFium-Flags abgeleitet).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum FallbackKind {
    Sans,
    Serif,
    Mono,
}

/// Liefert für nicht eingebettete PDF-Schriften eine Systemschrift.
///
/// Viele CAD-PDFs betten ihre Schrift nicht ein (z. B. `/BaseFont /CenturyGothic`
/// ohne `/FontFile2`). PDFium zeichnet dann Ersatz-Glyphen anhand der rohen
/// Zeichencodes – der Plan zeigt Buchstabensalat, obwohl die Textextraktion
/// korrekt ist. Mit einer echten Systemschrift, deren Glyphenreihenfolge der
/// Standard-Kodierung folgt (Arial, Times, Courier), rendert PDFium die Zeichen
/// richtig. Es wird nichts in das PDF geschrieben und nichts gespeichert.
struct SystemFontFallback {
    loaded: std::collections::HashMap<FallbackKind, (u64, Vec<u8>)>,
    next_id: u64,
}

impl SystemFontFallback {
    fn new() -> Self {
        Self {
            loaded: std::collections::HashMap::new(),
            next_id: 1,
        }
    }
}

/// Kandidatenpfade je Plattform und Kategorie (erster vorhandener gewinnt).
fn fallback_font_paths(kind: FallbackKind) -> &'static [&'static str] {
    if cfg!(target_os = "windows") {
        match kind {
            FallbackKind::Sans => &["C:\\Windows\\Fonts\\arial.ttf"],
            FallbackKind::Serif => &[
                "C:\\Windows\\Fonts\\times.ttf",
                "C:\\Windows\\Fonts\\arial.ttf",
            ],
            FallbackKind::Mono => &[
                "C:\\Windows\\Fonts\\cour.ttf",
                "C:\\Windows\\Fonts\\arial.ttf",
            ],
        }
    } else if cfg!(target_os = "macos") {
        match kind {
            FallbackKind::Sans => &[
                "/System/Library/Fonts/Supplemental/Arial.ttf",
                "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
            ],
            FallbackKind::Serif => &[
                "/System/Library/Fonts/Supplemental/Times New Roman.ttf",
                "/System/Library/Fonts/Supplemental/Arial.ttf",
            ],
            FallbackKind::Mono => &[
                "/System/Library/Fonts/Supplemental/Courier New.ttf",
                "/System/Library/Fonts/Supplemental/Arial.ttf",
            ],
        }
    } else {
        match kind {
            FallbackKind::Sans => &[
                "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
                "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
                "/usr/share/fonts/truetype/freefont/FreeSans.ttf",
            ],
            FallbackKind::Serif => &[
                "/usr/share/fonts/truetype/dejavu/DejaVuSerif.ttf",
                "/usr/share/fonts/truetype/liberation/LiberationSerif-Regular.ttf",
            ],
            FallbackKind::Mono => &[
                "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
                "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf",
            ],
        }
    }
}

impl PdfiumCustomFontProvider for SystemFontFallback {
    fn provide(
        &mut self,
        request: PdfiumCustomFontProviderRequest,
    ) -> Option<PdfiumCustomFontProviderResponse> {
        // Symbolschriften (z. B. Wingdings) nicht ersetzen – dafür hat PDFium
        // eigene Ersatzbehandlung; eine Textschrift würde die Symbole zerstören.
        if matches!(&request.character_set, PdfFontCharacterSet::Symbol) {
            return None;
        }

        let kind = if request.is_fixed_pitch {
            FallbackKind::Mono
        } else if request.is_serif {
            FallbackKind::Serif
        } else {
            FallbackKind::Sans
        };

        if !self.loaded.contains_key(&kind) {
            let mut data = None;
            for path in fallback_font_paths(kind) {
                if let Ok(bytes) = fs::read(path) {
                    if !bytes.is_empty() {
                        data = Some(bytes);
                        break;
                    }
                }
            }
            // Keine passende Systemschrift: PDFium nutzt seinen eingebauten Ersatz.
            let data = data?;
            let id = self.next_id;
            self.next_id += 1;
            self.loaded.insert(kind, (id, data));
        }

        let (id, data) = self.loaded.get(&kind)?;
        Some(PdfiumCustomFontProviderResponse {
            id: *id,
            font_face: request.font_face.clone(),
            character_set: request.character_set,
            data: data.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdfium_render::prelude::PdfFontWeight;

    fn request(fixed_pitch: bool, serif: bool) -> PdfiumCustomFontProviderRequest {
        PdfiumCustomFontProviderRequest {
            font_face: "CenturyGothic".to_string(),
            character_set: PdfFontCharacterSet::Default,
            weight: PdfFontWeight::Weight400Normal,
            is_italic: false,
            is_fixed_pitch: fixed_pitch,
            is_serif: serif,
            is_cursive: false,
        }
    }

    #[test]
    fn test_fallback_provider_returns_font_and_caches() {
        let mut provider = SystemFontFallback::new();
        let Some(first) = provider.provide(request(false, false)) else {
            // Auf Systemen ohne passende Schrift ist "kein Ersatz" korrekt.
            return;
        };
        assert!(first.data.len() > 1000, "Schriftdatei geladen");
        assert_eq!(first.font_face, "CenturyGothic");
        let second = provider.provide(request(false, false)).expect("cached");
        assert_eq!(first.id, second.id, "gleiche Schrift -> gleiche ID");
        // Serif wird getrennt geladen (andere ID, sofern vorhanden).
        if let Some(serif) = provider.provide(request(false, true)) {
            assert_ne!(first.id, serif.id);
        }
    }

    #[test]
    fn test_fallback_provider_ignores_symbol() {
        let mut provider = SystemFontFallback::new();
        let mut req = request(false, false);
        req.character_set = PdfFontCharacterSet::Symbol;
        assert!(provider.provide(req).is_none());
    }
}
