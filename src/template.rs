//! Vorlagen-Verwaltung und Koordinaten-basierte Extraktion (Teach-In).

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::backend::{PageData, Word};
use crate::extract;

/// Ein normiertes Rechteck relativ zur Seitengröße (0.0 ..= 1.0).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct NormalizedRect {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

#[allow(dead_code)]
impl NormalizedRect {
    pub fn new(x0: f32, y0: f32, x1: f32, y1: f32) -> Self {
        Self {
            x0: x0.min(x1).clamp(0.0, 1.0),
            y0: y0.min(y1).clamp(0.0, 1.0),
            x1: x0.max(x1).clamp(0.0, 1.0),
            y1: y0.max(y1).clamp(0.0, 1.0),
        }
    }

    pub fn width(&self) -> f32 {
        self.x1 - self.x0
    }

    pub fn height(&self) -> f32 {
        self.y1 - self.y0
    }

    pub fn is_valid(&self) -> bool {
        self.width() > 0.002 && self.height() > 0.002
    }
}

/// Ein konfiguriertes Schriftfeld innerhalb einer Vorlage.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TemplateField {
    /// Eindeutiger Schlüssel, z. B. "zeichnung", "bezeichnung", "material", "oberflaeche"
    pub key: String,
    /// Anzeigename
    pub label: String,
    /// Koordinatenbereich auf der Seite
    pub rect: NormalizedRect,
}

/// Eine gelernte Zeichnungsvorlage (Teach-In).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Template {
    pub id: String,
    pub name: String,
    pub fields: Vec<TemplateField>,
}

#[allow(dead_code)]
impl Template {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        let id = format!(
            "{}_{}",
            sanitize_id(&name),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
        );
        Self {
            id,
            name,
            fields: Vec::new(),
        }
    }

    pub fn set_field(&mut self, key: &str, label: &str, rect: NormalizedRect) {
        if let Some(f) = self.fields.iter_mut().find(|f| f.key == key) {
            f.rect = rect;
            f.label = label.to_string();
        } else {
            self.fields.push(TemplateField {
                key: key.to_string(),
                label: label.to_string(),
                rect,
            });
        }
    }

    pub fn remove_field(&mut self, key: &str) {
        self.fields.retain(|f| f.key != key);
    }

    pub fn get_field(&self, key: &str) -> Option<&TemplateField> {
        self.fields.iter().find(|f| f.key == key)
    }

    /// Extrahiert alle konfigurierten Felder aus den Seitendaten.
    pub fn extract(&self, page: &PageData) -> Vec<(String, String)> {
        self.fields
            .iter()
            .map(|f| (f.key.clone(), extract_field_text(page, &f.rect)))
            .collect()
    }

    /// Bewertet, wie gut diese Vorlage zu einer Seite passt (Anzahl nicht-leerer Treffer).
    pub fn match_score(&self, page: &PageData) -> usize {
        self.fields
            .iter()
            .filter(|f| !extract_field_text(page, &f.rect).is_empty())
            .count()
    }
}

fn sanitize_id(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

/// Extrahiert und bereinigt den Text innerhalb eines normierten Rechtecks.
pub fn extract_field_text(page: &PageData, rect: &NormalizedRect) -> String {
    if page.width <= 0.0 || page.height <= 0.0 {
        return String::new();
    }

    let abs_x0 = rect.x0 * page.width;
    let abs_y0 = rect.y0 * page.height;
    let abs_x1 = rect.x1 * page.width;
    let abs_y1 = rect.y1 * page.height;

    // Zuerst Wörter finden, deren Zentrum in der Box liegt
    let mut words: Vec<&Word> = page
        .words
        .iter()
        .filter(|w| {
            let cx = (w.x0 + w.x1) * 0.5;
            let cy = (w.y0 + w.y1) * 0.5;
            cx >= abs_x0 && cx <= abs_x1 && cy >= abs_y0 && cy <= abs_y1
        })
        .collect();

    // Falls kein Zentrum passt, prüfen, ob Bounding Boxes überlappen
    if words.is_empty() {
        words = page
            .words
            .iter()
            .filter(|w| !(w.x1 < abs_x0 || w.x0 > abs_x1 || w.y1 < abs_y0 || w.y0 > abs_y1))
            .collect();
    }

    if words.is_empty() {
        return String::new();
    }

    // Wörter in natürlicher Leserichtung sortieren (Zeilenweise von oben nach unten, links nach rechts)
    words.sort_by(|a, b| {
        let y_diff = (a.y0 - b.y0).abs();
        let line_height = (a.size.max(b.size) * 0.5).max(3.5);
        if y_diff < line_height {
            a.x0.partial_cmp(&b.x0).unwrap_or(std::cmp::Ordering::Equal)
        } else {
            a.y0.partial_cmp(&b.y0).unwrap_or(std::cmp::Ordering::Equal)
        }
    });

    let raw_text = words
        .iter()
        .map(|w| w.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");

    clean_text(&raw_text)
}

/// Säubert den extrahierten Text (Leerzeichen, CID-Sonderzeichen, führende Trennzeichen).
pub fn clean_text(raw: &str) -> String {
    let mut s = raw.trim().to_string();
    if s.is_empty() {
        return s;
    }

    // CID-Dekodierung für PDFs ohne Unicode-CMap anwenden
    s = s
        .chars()
        .map(|c| match c as u32 {
            104 => 'Ü',
            108 => 'ä',
            124 => 'ö',
            129 => 'ü',
            131 => '°',
            137 => 'ß',
            147 => '±',
            161 => 'ø',
            240 => '×',
            _ => c,
        })
        .collect();

    // Mehrfache Leerzeichen zusammenfassen
    let mut clean = String::with_capacity(s.len());
    let mut last_was_space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            if !last_was_space {
                clean.push(' ');
                last_was_space = true;
            }
        } else {
            clean.push(c);
            last_was_space = false;
        }
    }

    let mut result = clean.trim().to_string();

    // Häufige führende Label-Präfixe entfernen, falls das Label mitmarkiert wurde
    let prefixes = [
        "Zeichnungsnummer:",
        "Zeichnungs-Nr.:",
        "Zeichnungs-Nr",
        "Zeichnung-Nr.:",
        "Zeichnung Nr:",
        "Drawing No.:",
        "Drawing No",
        "Bezeichnung:",
        "Benennung:",
        "Part Name:",
        "Description:",
        "Material:",
        "Werkstoff:",
        "Oberfläche:",
        "Oberflächenbehandlung:",
        "Surface Treatment:",
    ];

    for prefix in prefixes {
        if let Some(stripped) = result.strip_prefix(prefix) {
            result = stripped.trim().to_string();
            break;
        }
        if result.to_lowercase().starts_with(&prefix.to_lowercase()) {
            result = result[prefix.len()..].trim().to_string();
            break;
        }
    }

    result
}

/// Extrahiert Felder aus den Seiten unter Berücksichtigung von Vorlagen und Modi.
pub fn extract_with_mode(
    pages: &[PageData],
    templates: &[Template],
    template_id: Option<&str>,
) -> extract::Extraction {
    if pages.is_empty() {
        return extract::Extraction {
            values: Vec::new(),
            missing: Vec::new(),
        };
    }

    let first_page = &pages[0];

    // 1. Spezifische Vorlage gewählt?
    if let Some(id) = template_id {
        if id != "none" && id != "auto" {
            if let Some(tmpl) = templates.iter().find(|t| t.id == id) {
                return extract_using_template(pages, tmpl);
            }
        } else if id == "none" {
            return extract::extract(pages);
        }
    }

    // 2. Automatischer Modus: Vorlage mit höchster Übereinstimmung suchen
    let mut best_match: Option<(&Template, usize)> = None;
    for tmpl in templates {
        let score = tmpl.match_score(first_page);
        if score >= 2 {
            if let Some((_, best_score)) = best_match {
                if score > best_score {
                    best_match = Some((tmpl, score));
                }
            } else {
                best_match = Some((tmpl, score));
            }
        }
    }

    if let Some((tmpl, _)) = best_match {
        return extract_using_template(pages, tmpl);
    }

    // 3. Fallback: Standard-Heuristik
    extract::extract(pages)
}

fn extract_using_template(pages: &[PageData], tmpl: &Template) -> extract::Extraction {
    let first_page = &pages[0];
    let tmpl_values = tmpl.extract(first_page);
    let heuristic = extract::extract(pages);

    let mut combined: Vec<(String, String)> = Vec::new();

    for f in &extract::FIELDS {
        let val = tmpl_values
            .iter()
            .find(|(k, _)| k == f.key)
            .map(|(_, v)| v.clone())
            .filter(|v| !v.is_empty())
            .or_else(|| {
                heuristic
                    .values
                    .iter()
                    .find(|(k, _)| k == f.key)
                    .map(|(_, v)| v.clone())
            })
            .unwrap_or_default();
        combined.push((f.key.to_string(), val));
    }

    let missing = extract::FIELDS
        .iter()
        .filter(|f| {
            combined
                .iter()
                .find(|(k, _)| k == f.key)
                .map(|(_, v)| v.is_empty())
                .unwrap_or(true)
        })
        .map(|f| f.title.to_string())
        .collect();

    extract::Extraction {
        values: combined,
        missing,
    }
}

/// Pfad zur Vorlagendatei `templates.json`.
pub fn templates_path() -> PathBuf {
    let local = PathBuf::from("templates.json");
    if local.exists() {
        return local;
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let p = parent.join("templates.json");
            if p.exists() {
                return p;
            }
        }
    }

    local
}

/// Lädt alle gespeicherten Vorlagen aus `templates.json`.
pub fn load_templates() -> Vec<Template> {
    let path = templates_path();
    let Ok(data) = fs::read_to_string(&path) else {
        return Vec::new();
    };
    serde_json::from_str(&data).unwrap_or_default()
}

/// Speichert die Vorlagenliste dauerhaft in `templates.json`.
pub fn save_templates(templates: &[Template]) -> Result<(), String> {
    let path = templates_path();
    let json = serde_json::to_string_pretty(templates)
        .map_err(|e| format!("Fehler beim Serialisieren der Vorlagen: {e}"))?;
    fs::write(&path, json).map_err(|e| format!("Fehler beim Schreiben von {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalized_rect() {
        let r = NormalizedRect::new(0.8, 0.9, 0.7, 0.8);
        assert!((r.x0 - 0.7).abs() < 1e-4);
        assert!((r.x1 - 0.8).abs() < 1e-4);
        assert!(r.is_valid());
    }

    #[test]
    fn test_extract_field_text() {
        let words = vec![
            Word {
                text: "E232174".to_string(),
                x0: 100.0,
                y0: 100.0,
                x1: 150.0,
                y1: 115.0,
                size: 12.0,
                baseline: 112.0,
            },
            Word {
                text: "DRUCKBOLZEN".to_string(),
                x0: 100.0,
                y0: 150.0,
                x1: 200.0,
                y1: 165.0,
                size: 12.0,
                baseline: 162.0,
            },
        ];
        let page = PageData {
            width: 1000.0,
            height: 1000.0,
            words,
            hsegs: vec![],
            vsegs: vec![],
        };

        let rect_num = NormalizedRect::new(0.09, 0.09, 0.16, 0.12);
        assert_eq!(extract_field_text(&page, &rect_num), "E232174");

        let rect_bez = NormalizedRect::new(0.09, 0.14, 0.22, 0.17);
        assert_eq!(extract_field_text(&page, &rect_bez), "DRUCKBOLZEN");
    }

    #[test]
    fn test_render_sample_preview() {
        let pdfium = match crate::pdfium_loader::load_pdfium() {
            Ok(p) => p,
            Err(_) => return,
        };
        let sample = std::path::Path::new("Samples/e232174.pdf");
        if sample.exists() {
            let (w, h, rgba, pdata) = crate::backend::render_page_preview(&pdfium, sample, 0, 1000).unwrap();
            assert!(w >= 800);
            assert!(h > 0);
            assert_eq!(rgba.len(), (w * h * 4) as usize);
            assert!(!pdata.words.is_empty());
        }
    }

    #[test]
    fn test_template_save_load_and_mode() {
        let mut tmpl = Template::new("TestVorlage");
        tmpl.set_field("zeichnung", "Zeichnungsnummer", NormalizedRect::new(0.1, 0.1, 0.2, 0.2));
        tmpl.set_field("bezeichnung", "Bezeichnung", NormalizedRect::new(0.1, 0.2, 0.3, 0.3));

        let words = vec![
            Word {
                text: "DWG-999".to_string(),
                x0: 120.0,
                y0: 120.0,
                x1: 180.0,
                y1: 140.0,
                size: 14.0,
                baseline: 135.0,
            },
            Word {
                text: "FLANSCH".to_string(),
                x0: 120.0,
                y0: 220.0,
                x1: 200.0,
                y1: 240.0,
                size: 14.0,
                baseline: 235.0,
            },
        ];
        let page = PageData {
            width: 1000.0,
            height: 1000.0,
            words,
            hsegs: vec![],
            vsegs: vec![],
        };

        assert_eq!(tmpl.match_score(&page), 2);

        let ext = extract_with_mode(&[page], &[tmpl.clone()], Some(&tmpl.id));
        let num = ext.values.iter().find(|(k, _)| k == "zeichnung").map(|(_, v)| v.as_str());
        let bez = ext.values.iter().find(|(k, _)| k == "bezeichnung").map(|(_, v)| v.as_str());
        assert_eq!(num, Some("DWG-999"));
        assert_eq!(bez, Some("FLANSCH"));

        let json = serde_json::to_string(&vec![tmpl.clone()]).unwrap();
        let loaded: Vec<Template> = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "TestVorlage");
    }

}
