//! Auswertung des Schriftfelds.
//!
//! Die Zeichnungen verwenden unterschiedliche Formate (Hoch-/Querformat, verschiedene
//! Vorlagen). Deshalb wird nicht auf feste Positionen geprüft, sondern das Schriftfeld
//! als Raster gelesen: Beschriftung suchen, umgebende Zelle über die Rasterlinien
//! bestimmen und den Zellinhalt auslesen.

use crate::backend::{HSeg, PageData, VSeg, Word};

/// Die ausgelesenen Felder in Anzeigereihenfolge.
pub const FIELDS: [Field; 4] = [
    Field {
        key: "zeichnung",
        title: "Zeichnungsnummer",
        patterns: &[
            "zeichnungsnummer",
            "zeichnung-nr",
            "zeichnung nr",
            "zeichnungs-nr",
            "drawing no",
            "drawing number",
            "t-drawing no",
            "t drawing no",
        ],
    },
    Field {
        key: "bezeichnung",
        title: "Bezeichnung",
        patterns: &[
            "bezeichnung",
            "benennung",
            "part name",
            "designation",
            "description:",
        ],
    },
    Field {
        key: "material",
        title: "Material",
        patterns: &["material", "werkstoff"],
    },
    Field {
        key: "oberflaeche",
        title: "Oberflächenbehandlung",
        patterns: &[
            "oberflaechenbehandlung",
            "oberflaeche",
            "surface treatment",
            "execution",
        ],
    },
];

pub struct Field {
    pub key: &'static str,
    pub title: &'static str,
    patterns: &'static [&'static str],
}

/// Wörter, die ein Feld eindeutig kennzeichnen.
const QUALIFIERS: [&str; 15] = [
    "old", "alt", "alternativ", "altzeichnung", "neu", "new", "vor", "prev", "previous", "ersatz",
    "rev", "revision", "backup", "frueher", "früher",
];

/// Zusammengehörige Textblöcke einer Zeile.
#[derive(Clone, Debug)]
pub struct Cluster {
    pub text: String,
    pub words: Vec<Word>,
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub size: f32,
}

impl Cluster {
    fn from_words(words: Vec<Word>) -> Cluster {
        let text = join_words(&words);
        let x0 = words.iter().map(|w| w.x0).fold(f32::INFINITY, f32::min);
        let x1 = words.iter().map(|w| w.x1).fold(f32::NEG_INFINITY, f32::max);
        let y0 = words.iter().map(|w| w.y0).fold(f32::INFINITY, f32::min);
        let y1 = words.iter().map(|w| w.y1).fold(f32::NEG_INFINITY, f32::max);
        let size = words.iter().map(|w| w.size).fold(0.0f32, f32::max);
        Cluster {
            text,
            words,
            x0,
            y0,
            x1,
            y1,
            size,
        }
    }
}

/// Ergebnis einer Zeichnung.
pub struct Extraction {
    pub values: Vec<(String, String)>, // (Feldkey, Wert)
    pub missing: Vec<String>,          // Titel der nicht gefundenen Felder
}

/// Dekodiert Text einer PDF, die kein ToUnicode-CMap besitzt: pdfium liefert hier
/// den rohen CID-Wert als Unicode-Zeichen (U+00xx). Die Zeichensätze der
/// KI-Automation-Vorlage (CenturyGothic-Subset) haben die Sonderzeichen an
/// festen CIDs; die restlichen Zeichen stehen um 29 Codes tiefer als ihr
/// Unicode-Wert.
fn decode_cid_char(c: char) -> char {
    let cid = c as u32;
    match cid {
        104 => 'Ü',
        108 => 'ä',
        124 => 'ö',
        129 => 'ü',
        131 => '°',
        137 => 'ß',
        147 => '±',
        161 => 'ø',
        240 => '×',
        _ => {
            let v = cid + 29;
            if (32..127).contains(&v) {
                char::from_u32(v).unwrap_or(c)
            } else {
                c
            }
        }
    }
}

fn decode_cid_text(text: &str) -> String {
    text.chars().map(decode_cid_char).collect()
}

/// Kopie der Seite mit decodierten Textwörtern.
pub fn decode_page(page: &PageData) -> PageData {
    PageData {
        width: page.width,
        height: page.height,
        words: page
            .words
            .iter()
            .map(|w| {
                let mut w = w.clone();
                w.text = decode_cid_text(&w.text);
                w
            })
            .collect(),
        hsegs: page.hsegs.clone(),
        vsegs: page.vsegs.clone(),
    }
}

/// Normalisiert einen Materialwert für die Auswertungsliste.
///
/// Aus Angaben wie „16 MnCr5 / 1.7131 / einsatzhaerten / CASE HARDENED“ wird
/// nur die reine Werkstoffnummer übernommen, z. B. „1.7131".
pub fn normalize_material(raw: &str) -> String {
    let cleaned = clean_material_text(raw);
    if cleaned.is_empty() {
        return cleaned;
    }

    // Mit „ / “ getrennte Segmente: Den Segment, der die Werkstoffnummer trägt, übernehmen.
    let segments: Vec<&str> = cleaned.split('/').map(str::trim).filter(|s| !s.is_empty()).collect();
    for segment in segments {
        if let Some(number) = find_material_number(segment) {
            return number;
        }
    }

    // Ohne „ / “ reicht die reine Zahlenkennung (z. B. „1.0037 S235JR“ -> „1.0037“).
    find_material_number(&cleaned).unwrap_or(cleaned)
}

/// Normaliert einen Bezeichnungswert: Deutsche Angabe bevorzugen, englische
/// Übersetzung ignorieren (übernehmen, falls keine deutsche vorhanden ist).
///
/// Beispiel: „DRUCKBOLZEN / PRESSURE BOLT“ -> „DRUCKBOLZEN“.
pub fn normalize_designation(raw: &str) -> String {
    let cleaned = clean_material_text(raw);
    if cleaned.is_empty() {
        return cleaned;
    }
    let parts: Vec<&str> = cleaned.split('/').map(str::trim).filter(|s| !s.is_empty()).collect();
    if parts.len() <= 1 {
        cleaned
    } else {
        parts[0].to_string()
    }
}

/// Bereinigt einen Materialtext (Trimmen, mehrfaches Leerzeichen zusammenfassen).
fn clean_material_text(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut last_was_space = false;
    for c in raw.trim().chars() {
        if c.is_whitespace() {
            if !last_was_space {
                out.push(' ');
                last_was_space = true;
            }
        } else {
            out.push(c);
            last_was_space = false;
        }
    }
    out.trim().to_string()
}

/// Sucht die erste Werkstoffnummer (Zahlen mit Punkt, z. B. „1.7131“) im Text.
fn find_material_number(text: &str) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;
    while i < len {
        if chars[i].is_ascii_digit() {
            let start = i;
            while i < len && chars[i].is_ascii_digit() {
                i += 1;
            }
            // Zahlen mit Dezimalpunkt erkennen, z. B. "1.7131"
            if i + 1 < len && chars[i] == '.' && chars[i + 1].is_ascii_digit() {
                let number_start = start;
                let mut end = i + 1;
                while end < len && chars[end].is_ascii_digit() {
                    end += 1;
                }
                let before_ok = start == 0 || !chars[start - 1].is_ascii_alphanumeric();
                let after_ok = end >= len || !chars[end].is_ascii_alphanumeric();
                if before_ok && after_ok {
                    let number: String = chars[number_start..end].iter().collect();
                    return Some(number);
                }
            }
            continue;
        }
        i += 1;
    }
    None
}

/// Liest alle Felder einer PDF-Datei aus.
pub fn extract(pages: &[PageData]) -> Extraction {
    let mut best: Option<Vec<(String, String)>> = None;
    let mut best_score = 0usize;

    for page in pages {
        // Zuerst rohen Text versuchen (PDFs mit ToUnicode), dann CID-Dekodierung
        // (PDFs ohne ToUnicode). Nur wenn die Dekodierung mehr Felder liefert,
        // gewinnt sie – so kaputten keine decodierte Seite einen korrekten Text.
        for decoded in [false, true] {
            let owned;
            let p: &PageData = if decoded {
                owned = decode_page(page);
                &owned
            } else {
                page
            };
            let values = extract_page(p);
            let score = values.iter().filter(|(_, v)| !v.is_empty()).count();
            if score > best_score {
                best_score = score;
                best = Some(values);
            }
        }
    }

    let values = best.unwrap_or_default();
    let missing = FIELDS
        .iter()
        .filter(|f| {
            values
                .iter()
                .find(|(k, _)| *k == f.key)
                .map(|(_, v)| v.is_empty())
                .unwrap_or(true)
        })
        .map(|f| f.title.to_string())
        .collect();

    Extraction { values, missing }
}

/// Diagnosemodus: DRAWGRAB_DEBUG=1 setzen, um Kandidaten, Zellen und Auswahlen
/// auf stdout auszugeben.
fn dbg() -> bool {
    std::env::var_os("DRAWGRAB_DEBUG").is_some()
}

fn extract_page(page: &PageData) -> Vec<(String, String)> {
    let clusters = cluster_words(&page.words);
    let (hsegs, vsegs) = build_grid(page, &clusters);
    if dbg() {
        println!(
            "DEBUG: {} Cluster, {} H-Linien, {} V-Linien",
            clusters.len(),
            hsegs.len(),
            vsegs.len()
        );
        for h in hsegs.iter() {
            println!("DEBUG: H y={:.1} x={:.1}..{:.1}", h.y, h.x0, h.x1);
        }
        for v in vsegs.iter() {
            println!("DEBUG: V x={:.1} y={:.1}..{:.1}", v.x, v.y0, v.y1);
        }
    }

    // --- alle Beschriftungen sammeln -------------------------------------
    struct Candidate {
        field: &'static str,
        cluster: usize,
        /// Index des ersten Wortes der Beschriftung im Cluster
        start: usize,
        /// Index des letzten Wortes der Beschriftung
        end: usize,
    }

    let mut candidates: Vec<Candidate> = Vec::new();
    for (ci, cluster) in clusters.iter().enumerate() {
        let norm_text = normalize(&cluster.text);
        let is_label = cluster.text.trim_end().ends_with(':')
            || FIELDS.iter().any(|f| {
                f.patterns.iter().any(|p| norm_text.contains(p))
            });
        if !is_label {
            continue;
        }
        for field in FIELDS.iter() {
            if let Some((start, end)) = match_field(cluster, field) {
                if dbg() {
                    println!(
                        "DEBUG: Kandidat {} in Cluster #{ci} {:?} words[{}..={}] = {:?}",
                        field.key,
                        cluster.text,
                        start,
                        end,
                        join_words(&cluster.words[start..=end])
                    );
                }
                candidates.push(Candidate {
                    field: field.key,
                    cluster: ci,
                    start,
                    end,
                });
            }
        }
    }
    if candidates.is_empty() {
        return FIELDS
            .iter()
            .map(|f| (f.key.to_string(), String::new()))
            .collect();
    }

    // Beschriftungsgröße als Maßstab für den Zeilenabstand
    let mut sizes: Vec<f32> = candidates
        .iter()
        .map(|c| {
            let cluster = &clusters[c.cluster];
            cluster.words[c.end].y1 - cluster.words[c.start].y0
        })
        .collect();
    sizes.sort_by(|a, b| a.total_cmp(b));
    let label_size = sizes.get(sizes.len() / 2).copied().unwrap_or(10.0);
    let band = 3.0 * label_size.max(4.0);

    let mut out: Vec<(String, String)> = Vec::new();
    for field in FIELDS.iter() {
        let mut best: Option<(usize, usize, String)> = None;

        for (ci, cand) in candidates.iter().enumerate() {
            if cand.field != field.key {
                continue;
            }
            let cluster = &clusters[cand.cluster];
            let label_words = &cluster.words[cand.start..=cand.end];
            let box_ = (
                label_words.iter().map(|w| w.x0).fold(f32::INFINITY, f32::min),
                label_words.iter().map(|w| w.y0).fold(f32::INFINITY, f32::min),
                label_words.iter().map(|w| w.x1).fold(f32::NEG_INFINITY, f32::max),
                label_words.iter().map(|w| w.y1).fold(f32::NEG_INFINITY, f32::max),
            );

            let Some((left, right, _top, bottom)) = cell_of(box_, &hsegs, &vsegs) else {
                if dbg() {
                    println!(
                        "DEBUG:   {} -> KEINE ZELLE für Label-Box ({:.1},{:.1})-({:.1},{:.1})",
                        field.key, box_.0, box_.1, box_.2, box_.3
                    );
                }
                continue;
            };
            if dbg() {
                println!(
                    "DEBUG:   {} Zelle l={left:.1} r={right:.1} t={:.1} b={bottom:.1}",
                    field.key, box_.3
                );
            }

            // Kandidateninhalte der Zelle einsammeln
            let mut items: Vec<usize> = Vec::new();
            for (ii, item) in clusters.iter().enumerate() {
                if ii == cand.cluster {
                    continue;
                }
                if item.y1 <= box_.3 + 1.0 || item.y0 >= bottom - 1.0 {
                    continue;
                }
                if item.x1 < left - 1.0 || item.x0 > right + 1.0 {
                    continue;
                }
                items.push(ii);
            }
            // Rest des Beschriftungs-Clusters (Text vor dem Label) gehört nicht zum Wert
            items.sort_by(|a, b| {
                clusters[*a]
                    .y0
                    .total_cmp(&clusters[*b].y0)
                    .then_with(|| {
                        clusters[*a]
                            .x0
                            .total_cmp(&clusters[*b].x0)
                    })
            });

            // Fremde Beschriftungen in der Zelle (endet auf ':')
            let mut subs: Vec<(f32, f32, f32, f32)> = Vec::new();
            for other in clusters.iter() {
                for w in other.words.iter() {
                    if !w.text.trim_end().ends_with(':') {
                        continue;
                    }
                    if w.x0 < left - 1.0 || w.x1 > right + 1.0 || w.y0 < box_.3 - 1.0 {
                        continue;
                    }
                    if (w.x0 - box_.0).abs() < 2.0 && (w.y0 - box_.1).abs() < 2.0 {
                        continue; // eigene Beschriftung
                    }
                    subs.push((w.x0, w.y0, w.x1, w.y1));
                }
            }

            let mut texts: Vec<String> = Vec::new();
            let items_debug = items
                .iter()
                .map(|ii| clusters[*ii].text.as_str())
                .collect::<Vec<_>>()
                .join(" || ");
            for ii in items {
                let item = &clusters[ii];
                // Text, der auf der Grundlinie einer fremden Beschriftung sitzt, gehört ihr
                let mut owner = false;
                for sub in subs.iter() {
                    let baseline = sub.3;
                    if item.y0 <= baseline + 2.0 && item.y1 >= baseline - 2.0 && sub.2 <= item.x0 + 2.0
                    {
                        owner = true;
                        break;
                    }
                }
                if owner {
                    break;
                }
                // Fremde Beschriftung (Text endet mit ':') gehört nicht zum Wert
                if item.text.trim_end().ends_with(':') {
                    continue;
                }
                // Wort mit ':' abtrennen (nur Teil vor dem Doppelpunkt übernehmen)
                if let Some(pos) = item
                    .words
                    .iter()
                    .position(|w| w.text.trim_end().ends_with(':'))
                {
                    if pos > 0 {
                        texts.push(join_words(&item.words[..pos]));
                    }
                    break;
                }
                texts.push(item.text.trim().to_string());
            }

            let texts: Vec<String> = texts
                .into_iter()
                .filter(|t| !t.is_empty() && !t.trim_matches(['.', '\u{b7}', '-', '\u{2013}', '\u{2014}']).is_empty())
                .collect();
            let value = texts.join(" / ");
            if dbg() {
                println!(
                    "DEBUG:   {} Items=[{}] -> Wert={:?}",
                    field.key,
                    items_debug,
                    value
                );
            }

            // Unterstützung: Anzahl anderer Feldbeschriftungen in derselben Zeile
            let support = candidates
                .iter()
                .enumerate()
                .filter(|(oi, other)| {
                    *oi != ci && other.field != cand.field && {
                        let oc = &clusters[other.cluster];
                        let ow = &oc.words[other.start];
                        (ow.y0 - box_.1).abs() <= band
                    }
                })
                .count();

            let score = (support, value.len());
            if best.as_ref().map(|b| score > (b.0, b.1)).unwrap_or(true) {
                best = Some((support, value.len(), value));
            }
        }

        let value = best.map(|(_, _, v)| v).unwrap_or_default();
        // Zeichnungsnummern wie "T26043 2" enthalten meistens ein Split-Artefakt
        // (Leerzeichen zwischen den Ziffern) – daraus wird die saubere Nummer gemacht.
        let value = if field.key == "zeichnung" {
            value.chars().filter(|c| !c.is_whitespace()).collect()
        } else {
            value
        };
        out.push((field.key.to_string(), value));
    }

    out
}

/// Sucht die Beschriftung eines Feldes in einem Cluster.
/// Liefert (Startindex, Endindex) der Wörter, die zur Beschriftung gehören.
fn match_field(cluster: &Cluster, field: &Field) -> Option<(usize, usize)> {
    // Text mit abstandsbasierter Verbindung, damit auch künstlich getrennte
    // Wörter (z. B. "Zeichnungs" + "nummer") im Muster gefunden werden.
    let normalized: Vec<String> = cluster.words.iter().map(|w| normalize(&w.text)).collect();
    let mut full = String::new();
    let mut spans: Vec<(usize, usize)> = Vec::new(); // Bytebereich je Wort
    for (i, w) in normalized.iter().enumerate() {
        if i > 0 {
            let prev = &cluster.words[i - 1];
            let gap = cluster.words[i].x0 - prev.x1;
            let limit = 0.4 * cluster.words[i].size.max(prev.size);
            if gap > limit {
                full.push(' ');
            }
        }
        let start = full.len();
        full.push_str(w);
        spans.push((start, full.len()));
    }

    for pattern in field.patterns.iter() {
        let Some(pos) = full.find(pattern) else {
            continue;
        };
        // Wortindex, in dem die Fundstelle beginnt (nicht das ganze Cluster!)
        let start = spans.iter().position(|(s, e)| pos < *e && pos >= *s)?;
        // Qualifier davor (z. B. "OLD DRAWING No:") ausschließen
        if start > 0 {
            let previous = normalized[start - 1].trim_matches([':', ',', '.']);
            if QUALIFIERS.iter().any(|q| previous == *q) {
                continue;
            }
        }
        return Some((start, cluster.words.len() - 1));
    }
    None
}

/// Gruppiert Wörter zu Textblöcken (Zeilen, die durch größere Abstände getrennt sind).
fn cluster_words(words: &[Word]) -> Vec<Cluster> {
    if words.is_empty() {
        return Vec::new();
    }

    // Zeilen anhand der Grundlinie bilden
    let mut indexed: Vec<(usize, &Word)> = words.iter().enumerate().collect();
    indexed.sort_by(|a, b| {
        a.1.baseline
            .total_cmp(&b.1.baseline)
            .then_with(|| a.1.x0.total_cmp(&b.1.x0))
    });

    let mut rows: Vec<Vec<(usize, &Word)>> = Vec::new();
    for (i, w) in indexed {
        let tol = (0.3 * w.size).max(1.0);
        match rows.last_mut() {
            Some(row) if (w.baseline - row[0].1.baseline).abs() <= tol => row.push((i, w)),
            _ => rows.push(vec![(i, w)]),
        }
    }

    // Innerhalb einer Zeile an großen Abständen trennen
    let mut clusters = Vec::new();
    for mut row in rows {
        row.sort_by(|a, b| a.1.x0.total_cmp(&b.1.x0));
        let mut current: Vec<Word> = Vec::new();
        for (_, w) in row {
            if let Some(prev) = current.last() {
                let gap = w.x0 - prev.x1;
                let limit = (12.0f32).max(2.2 * w.size);
                if gap > limit {
                    clusters.push(Cluster::from_words(std::mem::take(&mut current)));
                }
            }
            current.push(w.clone());
        }
        if !current.is_empty() {
            clusters.push(Cluster::from_words(current));
        }
    }
    clusters
}

/// Entfernt Glyphenstriche (Schreibschrift) aus der Linienerkennung.
fn build_grid(page: &PageData, clusters: &[Cluster]) -> (Vec<HSeg>, Vec<VSeg>) {
    let mut sizes: Vec<f32> = clusters.iter().map(|c| c.size).filter(|s| *s > 0.0).collect();
    sizes.sort_by(|a, b| a.total_cmp(b));
    let median = sizes.get(sizes.len() / 2).copied().unwrap_or(8.0);
    let min_len = (10.0f32).max(2.2 * median);

    let hsegs: Vec<HSeg> = page
        .hsegs
        .iter()
        .copied()
        .filter(|s| (s.x1 - s.x0) >= min_len && !inside_text(s.y, s.x0, s.x1, clusters))
        .collect();
    let vsegs: Vec<VSeg> = page
        .vsegs
        .iter()
        .copied()
        .filter(|s| (s.y1 - s.y0) >= min_len && !inside_text_v(s.x, s.y0, s.y1, clusters))
        .collect();

    (hsegs, vsegs)
}

fn inside_text(y: f32, x0: f32, x1: f32, clusters: &[Cluster]) -> bool {
    clusters.iter().any(|c| {
        x0 >= c.x0 - 2.0 && x1 <= c.x1 + 2.0 && y >= c.y0 - 2.0 && y <= c.y1 + 2.0
    })
}

fn inside_text_v(x: f32, y0: f32, y1: f32, clusters: &[Cluster]) -> bool {
    clusters.iter().any(|c| {
        x >= c.x0 - 2.0 && x <= c.x1 + 2.0 && y0 >= c.y0 - 2.0 && y1 <= c.y1 + 2.0
    })
}

/// Bestimmt die Zelle, in der eine Beschriftung liegt.
fn cell_of(
    b: (f32, f32, f32, f32),
    hsegs: &[HSeg],
    vsegs: &[VSeg],
) -> Option<(f32, f32, f32, f32)> {
    let cx = (b.0 + b.2) / 2.0;
    let cy = (b.1 + b.3) / 2.0;

    let left = vsegs
        .iter()
        .filter(|v| v.x <= b.0 + 1.0 && v.y0 - 1.0 <= cy && cy <= v.y1 + 1.0)
        .map(|v| v.x)
        .fold(None, |acc: Option<f32>, x| Some(acc.map_or(x, |a| a.max(x))));

    let right = vsegs
        .iter()
        .filter(|v| v.x >= b.2 - 1.0 && v.y0 - 1.0 <= cy && cy <= v.y1 + 1.0)
        .map(|v| v.x)
        .fold(None, |acc: Option<f32>, x| Some(acc.map_or(x, |a| a.min(x))));

    let top = hsegs
        .iter()
        .filter(|h| h.y <= b.1 + 1.0 && h.x0 - 1.0 <= cx && cx <= h.x1 + 1.0)
        .map(|h| h.y)
        .fold(None, |acc: Option<f32>, y| Some(acc.map_or(y, |a| a.max(y))));

    let bottom = hsegs
        .iter()
        .filter(|h| h.y >= b.3 - 1.0 && h.x0 - 1.0 <= cx && cx <= h.x1 + 1.0)
        .map(|h| h.y)
        .fold(None, |acc: Option<f32>, y| Some(acc.map_or(y, |a| a.min(y))));

    match (left, right, top, bottom) {
        (Some(l), Some(r), Some(t), Some(bo)) => Some((l, r, t, bo)),
        (Some(l), Some(r), None, Some(bo)) => Some((l, r, b.1, bo)),
        _ => None,
    }
}

/// Fügt Wörter einer Zeile zusammen. Ein kleiner Abstand zwischen zwei
/// Wörtern bedeutet, dass pdfium ein Wort künstlich getrennt hat (z. B.
/// "T26002" + "4"); dann wird kein Leerzeichen eingefügt.
fn join_words(words: &[Word]) -> String {
    let mut out = String::new();
    let mut prev: Option<&Word> = None;
    for w in words {
        if let Some(p) = prev {
            let gap = w.x0 - p.x1;
            let limit = 0.4 * w.size.max(p.size);
            if gap > limit {
                out.push(' ');
            }
        }
        out.push_str(&w.text);
        prev = Some(w);
    }
    out
}

/// Kleinbuchstaben und Umlaute vereinheitlichen.
/// Umlaute werden zu den Zweibuchstabenformen (ae/oe/ue/ss), damit sie zu den
/// Mustern wie "oberflaechenbehandlung" passen.
fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.to_lowercase().chars() {
        match c {
            'ä' => out.push_str("ae"),
            'ö' => out.push_str("oe"),
            'ü' => out.push_str("ue"),
            'ß' => out.push_str("ss"),
            other => out.push(other),
        }
    }
    out
}

/// Diagnoseausgabe: zeigt Wörter, Rasterlinien und die gefundenen Felder.
pub fn debug_dump(page: &PageData) {
    if std::env::var_os("DRAWGRAB_WORDS").is_some() {
        let mut words: Vec<&Word> = page.words.iter().collect();
        words.sort_by(|a, b| a.baseline
            .total_cmp(&b.baseline)
            .then_with(|| a.x0.total_cmp(&b.x0)));
        let mut last_baseline = f32::NAN;
        let mut prev_x1 = f32::NAN;
        for w in words {
            let gap = if w.baseline == last_baseline { w.x0 - prev_x1 } else { f32::NAN };
            if w.baseline != last_baseline {
                println!();
                print!("ZEILE ");
                last_baseline = w.baseline;
            }
            print!(
                "W {:?} x={:.1}..{:.1} s={:.2} gap={:.1} |",
                w.text, w.x0, w.x1, w.size, gap
            );
            prev_x1 = w.x1;
        }
        println!();
    }
    let clusters = cluster_words(&page.words);
    let (hsegs, vsegs) = build_grid(page, &clusters);
    println!("== Seite {:.0} x {:.0}, {} Wörter, {} Cluster", page.width, page.height, page.words.len(), clusters.len());
    println!("== Raster: {} waagerecht, {} senkrecht", hsegs.len(), vsegs.len());
    let mut visible: Vec<&Cluster> = clusters.iter().collect();
    visible.sort_by(|a, b| a.y0.total_cmp(&b.y0));
    for c in visible {
        if c.x0 < page.width * 0.45 {
            continue;
        }
        println!(
            "CL y={:7.1}..{:7.1} x={:7.1}..{:7.1} s={:5.2} | {}",
            c.y0, c.y1, c.x0, c.x1, c.size, c.text
        );
    }
    let values = extract_page(page);
    for (key, value) in values {
        println!("FELD {key} = {value:?}");
    }
    let decoded = decode_page(page);
    let values = extract_page(&decoded);
    for (key, value) in values {
        println!("FELD(decodiert) {key} = {value:?}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_material() {
        // Vollstaendige Angabe mit Werkstoffnummer -> nur die Nummer
        assert_eq!(normalize_material("16 MnCr5 / 1.7131"), "1.7131");
        assert_eq!(normalize_material("16 MnCr 5 / 1.7131"), "1.7131");
        assert_eq!(
            normalize_material("16 MnCr 5 / 1.7131 / einsatzhaerten / CASE HARDENED"),
            "1.7131"
        );
        assert_eq!(normalize_material("Ck 45 / 1.1191 / badnitriert / BATH NITRIDED"), "1.1191");
        assert_eq!(normalize_material("42 CrMo 4 / 1.7225 / gasnitriert / GAS NITRIDED"), "1.7225");
        assert_eq!(normalize_material("AlCuBiPb / 3.1655"), "3.1655");
        assert_eq!(normalize_material("AlMgSi 0, 5 / 3.3206"), "3.3206");
        assert_eq!(normalize_material("X 5 CrNi 18 10 / 1.4301"), "1.4301");
        // Reine Zahlenkennung ohne weitere Angabe
        assert_eq!(normalize_material("1.0037 S235JR"), "1.0037");
    }

    #[test]
    fn test_normalize_material_keep_plain_values() {
        assert_eq!(normalize_material("PA6 black PA6 schwarz"), "PA6 black PA6 schwarz");
        assert_eq!(normalize_material("Aluminium-Flachmaterial"), "Aluminium-Flachmaterial");
        assert_eq!(normalize_material("Material <nicht festgelegt>"), "Material <nicht festgelegt>");
        assert_eq!(normalize_material(""), "");
    }

    #[test]
    fn test_normalize_designation() {
        // Deutsche Angabe zuerst, englische Uebersetzung ignorieren
        assert_eq!(normalize_designation("DRUCKBOLZEN / PRESSURE BOLT"), "DRUCKBOLZEN");
        assert_eq!(normalize_designation("FUEHRUNG / GUIDEWAY"), "FUEHRUNG");
        assert_eq!(normalize_designation("RUNDFEDER / ROUND SPRING"), "RUNDFEDER");
        assert_eq!(normalize_designation("ANSCHLUSSPLATTE / CONNECTING PLATE"), "ANSCHLUSSPLATTE");
        // Reine englische Angabe ohne deutsche Alternative -> englisch uebernehmen
        assert_eq!(normalize_designation("PRESSURE BOLT"), "PRESSURE BOLT");
        assert_eq!(normalize_designation("Kunstst. \"S\" gruen"), "Kunstst. \"S\" gruen");
    }
}
