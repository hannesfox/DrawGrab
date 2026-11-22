//! DrawGrab – Schriftfelder technischer Zeichnungen (PDF) auslesen.
//!
//! Bedienung: PDF-Dateien (oder ganze Ordner) in das Fenster ziehen, "Alle auswerten"
//! drücken, die Tabelle prüfen und als CSV exportieren.

mod app;
mod backend;
mod extract;
mod pdfium_loader;

use std::path::{Path, PathBuf};

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() > 2 && args[1] == "--debug" {
        let file = PathBuf::from(&args[2]);
        let pdfium = match pdfium_loader::load_pdfium() {
            Ok(p) => p,
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        };
        let pages = backend::load_pages(&pdfium, &file).expect("PDF nicht lesbar");
        extract::debug_dump(&pages[0]);
        return;
    }

    if args.len() > 1 && args[1] == "--cli" {
        let folders: Vec<PathBuf> = args[2..].iter().map(PathBuf::from).collect();
        cli(folders);
        return;
    }

    app::run();
}

/// Kommandozeilenmodus: wertet alle PDFs der angegebenen Ordner aus und gibt sie aus.
fn cli(folders: Vec<PathBuf>) {
    let mut files: Vec<PathBuf> = Vec::new();
    for folder in &folders {
        collect_pdfs(folder, &mut files);
    }
    files.sort();

    let pdfium = match pdfium_loader::load_pdfium() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("PDFium konnte nicht geladen werden: {e}");
            std::process::exit(1);
        }
    };

    let mut found = 0;
    let mut total = 0;
    for file in &files {
        total += 1;
        match backend::load_pages(&pdfium, file) {
            Ok(pages) => {
                let result = extract::extract(&pages);
                let values: Vec<String> = result
                    .values
                    .iter()
                    .map(|(k, v)| {
                        let title = extract::FIELDS
                            .iter()
                            .find(|f| f.key == k)
                            .map(|f| f.title)
                            .unwrap_or(k);
                        format!("{title}={v}")
                    })
                    .collect();
                println!("{}\n    {}", file.display(), values.join(" | "));
                found += 1;
            }
            Err(e) => println!("{}\n    FEHLER: {e}", file.display()),
        }
    }
    println!("\n{found}/{total} Dateien ausgewertet");
}

/// Collectiert alle PDF-Dateien eines Ordners (rekursiv).
fn collect_pdfs(dir: &Path, out: &mut Vec<PathBuf>) {
    if dir.is_file() {
        if dir
            .extension()
            .map(|e| e.eq_ignore_ascii_case("pdf"))
            .unwrap_or(false)
        {
            out.push(dir.to_path_buf());
        }
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_pdfs(&path, out);
        } else if path
            .extension()
            .map(|e| e.eq_ignore_ascii_case("pdf"))
            .unwrap_or(false)
        {
            out.push(path);
        }
    }
}
