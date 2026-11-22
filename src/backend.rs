//! PDF-Backend auf Basis von PDFium.
//!
//! Liefert für jede Seite die Positionen aller Textwörter sowie die waagerechten
//! und senkrechten Linien (Rasterlinien des Schriftfelds) in einem Koordinatensystem
//! mit Ursprung oben links (y wächst nach unten), wie es die weitere Auswertung erwartet.

use std::path::Path;

use pdfium_render::prelude::*;

/// Ein Textwort mit Position auf der Seite.
#[derive(Clone, Debug)]
pub struct Word {
    pub text: String,
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    /// Höhe des Schriftbilds (x-Höhe) - dient als Größenmaß.
    pub size: f32,
    /// Grundlinie des Wortes
    pub baseline: f32,
}

impl Word {
    #[allow(dead_code)]
    pub fn width(&self) -> f32 {
        self.x1 - self.x0
    }
}

/// Waagerechte Linie: y = Konstante, x von x0 bis x1.
#[derive(Clone, Copy, Debug)]
pub struct HSeg {
    pub y: f32,
    pub x0: f32,
    pub x1: f32,
}

/// Senkrechte Linie: x = Konstante, y von y0 bis y1.
#[derive(Clone, Copy, Debug)]
pub struct VSeg {
    pub x: f32,
    pub y0: f32,
    pub y1: f32,
}

/// Die ausgewerteten Grunddaten einer Seite.
pub struct PageData {
    pub width: f32,
    pub height: f32,
    pub words: Vec<Word>,
    pub hsegs: Vec<HSeg>,
    pub vsegs: Vec<VSeg>,
}

#[derive(Clone, Debug)]
struct CharInfo {
    text: String,
    x0: f32,
    x1: f32,
    /// Oberkante (y wächst nach unten)
    y0: f32,
    /// Unterkante
    y1: f32,
    /// Schriftgröße
    size: f32,
    /// Grundlinie
    baseline: f32,
}

/// Lädt alle Seiten einer PDF-Datei.
pub fn load_pages(pdfium: &Pdfium, path: &Path) -> Result<Vec<PageData>, String> {
    let document = pdfium
        .load_pdf_from_file(path, None)
        .map_err(|e| format!("PDF konnte nicht geöffnet werden: {e}"))?;

    let mut pages = Vec::new();
    let count = document.pages().len();
    for index in 0..count {
        match page_data(&document, index) {
            Ok(data) => pages.push(data),
            Err(_) => continue,
        }
    }
    Ok(pages)
}

fn page_data(document: &PdfDocument, index: i32) -> Result<PageData, String> {
    let page = document
        .pages()
        .get(index)
        .map_err(|e| format!("Seite {index} nicht lesbar: {e}"))?;

    let size = page.page_size();
    let page_h = size.height().value;
    let page_w = size.width().value;

    let words = extract_words(&page, page_h)?;
    let (hsegs, vsegs) = extract_segments(&page, page_h);

    Ok(PageData {
        width: page_w,
        height: page_h,
        words,
        hsegs,
        vsegs,
    })
}

fn extract_words(page: &PdfPage, page_h: f32) -> Result<Vec<Word>, String> {
    let text_page = page.text().map_err(|e| format!("Text nicht lesbar: {e}"))?;

    let mut chars: Vec<CharInfo> = Vec::new();
    for ch in text_page.chars().iter() {
        let ch = ch;
        let Some(text) = ch.unicode_string() else {
            continue;
        };
        if text.is_empty() {
            continue;
        }
        let bounds = match ch.tight_bounds() {
            Ok(b) => b,
            Err(_) => continue,
        };
        let font_size = ch.scaled_font_size().value;
        // PDF-Koordinaten (Ursprung unten links) -> oben links
        let y_top = page_h - bounds.top().value;
        let y_bottom = page_h - bounds.bottom().value;
        let baseline = match ch.origin() {
            Ok((_, origin_y)) => page_h - origin_y.value,
            Err(_) => y_bottom,
        };
        chars.push(CharInfo {
            text,
            x0: bounds.left().value,
            x1: bounds.right().value,
            y0: y_top,
            y1: y_bottom,
            size: font_size,
            baseline,
        });
    }

    Ok(chars_to_words(&mut chars))
}

/// Gruppiert Zeichen zu Zeilen und anschließend zu Wörtern.
fn chars_to_words(chars: &mut [CharInfo]) -> Vec<Word> {
    chars.sort_by(|a, b| {
        a.baseline
            .partial_cmp(&b.baseline)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.x0.partial_cmp(&b.x0).unwrap_or(std::cmp::Ordering::Equal))
    });

    // 1) Zeichen in Zeilen (gleiche Grundlinie) gruppieren
    let mut rows: Vec<Vec<CharInfo>> = Vec::new();
    for ch in chars.iter() {
        let tol = (0.3 * ch.size).max(1.0);
        match rows.last_mut() {
            Some(row) if (ch.baseline - row[0].baseline).abs() <= tol => row.push(ch.clone()),
            _ => rows.push(vec![ch.clone()]),
        }
    }

    // 2) Zeichen in Wörter zerlegen (an echten Leerzeichen bzw. grossen Abständen)
    let mut words = Vec::new();
    for mut row in rows {
        row.sort_by(|a, b| a.x0.partial_cmp(&b.x0).unwrap_or(std::cmp::Ordering::Equal));
        let mut current: Vec<&CharInfo> = Vec::new();
        for ch in row.iter() {
            if ch.text.chars().all(|c| c.is_whitespace()) {
                push_word(&mut words, &mut current);
                continue;
            }
            if let Some(prev) = current.last() {
                let gap = ch.x0 - prev.x1;
                let tol = (1.2 * ch.size).max(2.0);
                if gap > tol {
                    push_word(&mut words, &mut current);
                }
            }
            current.push(ch);
        }
        push_word(&mut words, &mut current);
    }
    words
}

fn push_word(words: &mut Vec<Word>, current: &mut Vec<&CharInfo>) {
    if current.is_empty() {
        return;
    }
    let text: String = current.iter().map(|c| c.text.as_str()).collect();
    let x0 = current.iter().map(|c| c.x0).fold(f32::INFINITY, f32::min);
    let x1 = current.iter().map(|c| c.x1).fold(f32::NEG_INFINITY, f32::max);
    let y0 = current.iter().map(|c| c.y0).fold(f32::INFINITY, f32::min);
    let y1 = current.iter().map(|c| c.y1).fold(f32::NEG_INFINITY, f32::max);
    let size = current
        .iter()
        .map(|c| (c.y1 - c.y0).max(c.size * 0.55))
        .fold(0.0f32, f32::max);
    let baseline = current
        .iter()
        .map(|c| c.baseline)
        .fold(f32::NEG_INFINITY, f32::max);
    words.push(Word {
        text,
        x0,
        y0,
        x1,
        y1,
        size,
        baseline,
    });
    current.clear();
}

/// Liest alle Vektorpfade der Seite und gewinnt daraus waagerechte/senkrechte Linien.
fn extract_segments(page: &PdfPage, page_h: f32) -> (Vec<HSeg>, Vec<VSeg>) {
    let mut hsegs = Vec::new();
    let mut vsegs = Vec::new();

    for object in page.objects().iter() {
        let PdfPageObject::Path(path) = object else {
            continue;
        };

        // Die Pfadpunkte liegen im Koordinatensystem des Objekts; die Objekt-Matrix
        // bildet sie auf die Seite ab. Ohne diese Transformierung sind die Linien
        // (bei manchen CAD-Exporten) weit außerhalb der Seite.
        let matrix = path.matrix().unwrap_or(PdfMatrix::IDENTITY);
        let segments = path.segments().transform(matrix);

        let mut previous: Option<(f32, f32)> = None;
        for segment in segments.iter() {
            let (x, y) = segment.point();
            let point = (x.value, page_h - y.value);

            match segment.segment_type() {
                PdfPathSegmentType::MoveTo => previous = Some(point),
                PdfPathSegmentType::LineTo | PdfPathSegmentType::BezierTo => {
                    if let Some((px, py)) = previous {
                        add_segment(px, py, point.0, point.1, &mut hsegs, &mut vsegs);
                    }
                    previous = Some(point);
                }
                _ => previous = None,
            }
        }
    }

    (merge_h(&mut hsegs), merge_v(&mut vsegs))
}

fn add_segment(x0: f32, y0: f32, x1: f32, y1: f32, hsegs: &mut Vec<HSeg>, vsegs: &mut Vec<VSeg>) {
    if (y0 - y1).abs() < 0.6 && (x1 - x0).abs() > 0.6 {
        hsegs.push(HSeg {
            y: y0,
            x0: x0.min(x1),
            x1: x0.max(x1),
        });
    } else if (x0 - x1).abs() < 0.6 && (y1 - y0).abs() > 0.6 {
        vsegs.push(VSeg {
            x: x0,
            y0: y0.min(y1),
            y1: y0.max(y1),
        });
    }
}

fn merge_h(segs: &mut Vec<HSeg>) -> Vec<HSeg> {
    segs.sort_by(|a, b| a.y.partial_cmp(&b.y).unwrap_or(std::cmp::Ordering::Equal));
    let mut out: Vec<HSeg> = Vec::new();
    for s in segs.iter() {
        let mut merged = false;
        for o in out.iter_mut() {
            if (o.y - s.y).abs() < 0.8 && !(s.x1 < o.x0 - 1.0 || s.x0 > o.x1 + 1.0) {
                o.x0 = o.x0.min(s.x0);
                o.x1 = o.x1.max(s.x1);
                merged = true;
                break;
            }
        }
        if !merged {
            out.push(*s);
        }
    }
    out
}

fn merge_v(segs: &mut Vec<VSeg>) -> Vec<VSeg> {
    segs.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));
    let mut out: Vec<VSeg> = Vec::new();
    for s in segs.iter() {
        let mut merged = false;
        for o in out.iter_mut() {
            if (o.x - s.x).abs() < 0.8 && !(s.y1 < o.y0 - 1.0 || s.y0 > o.y1 + 1.0) {
                o.y0 = o.y0.min(s.y0);
                o.y1 = o.y1.max(s.y1);
                merged = true;
                break;
            }
        }
        if !merged {
            out.push(*s);
        }
    }
    out
}
