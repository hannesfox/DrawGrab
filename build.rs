//! Lädt die zur Laufzeit benötigte PDFium-Bibliothek herunter und bettet sie in das
//! Programm ein. Die Version ist bewusst exakt die, für die `pdfium-render` die
//! Bindings mitbringt (8125).

use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

const PDFIUM_VERSION: &str = "8125";
const BASE_URL: &str = "https://github.com/bblanchon/pdfium-binaries/releases/download";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=DRAWGRAB_PDFIUM_LIB");

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let target = env::var("TARGET").unwrap_or_default();

    // Selbst bereitgestellte Bibliothek verwenden
    if let Ok(custom) = env::var("DRAWGRAB_PDFIUM_LIB") {
        let path = PathBuf::from(&custom);
        if path.exists() {
            emit_library(&path, &out_dir);
            return;
        }
        panic!("DRAWGRAB_PDFIUM_LIB zeigt auf eine nicht existierende Datei: {custom}");
    }

    let asset = asset_name(&target);
    let url = format!("{BASE_URL}/chromium%2F{PDFIUM_VERSION}/{asset}");
    let archive = out_dir.join("pdfium-archive.tgz");

    if !archive.exists() {
        println!("cargo:warning=PDFium wird heruntergeladen: {url}");
        download(&url, &archive);
    }

    let library = extract_library(&archive, &out_dir)
        .expect("PDFium konnte aus dem Archiv nicht entpackt werden");
    emit_library(&library, &out_dir);
}

/// Name der Release-Datei für die Zielplattform.
fn asset_name(target: &str) -> String {
    let (os, arch) = if target.contains("apple-darwin") {
        ("mac", if target.contains("aarch64") { "arm64" } else { "x64" })
    } else if target.contains("windows") {
        ("win", if target.contains("aarch64") { "arm64" } else { "x64" })
    } else if target.contains("linux") {
        ("linux", if target.contains("aarch64") { "arm64" } else { "x64" })
    } else {
        panic!("Plattform {target} wird nicht unterstützt");
    };
    format!("pdfium-{os}-{arch}.tgz")
}

fn download(url: &str, dest: &Path) {
    for (program, args) in [
        ("curl", vec!["-L", "-f", "--retry", "3", "-o"]),
        ("wget", vec!["-O"]),
    ] {
        let mut command = Command::new(program);
        command.args(&args);
        command.arg(dest);
        command.arg(url);
        command.arg("-q");
        command.arg("-q");
        if let Ok(status) = command.status() {
            if status.success() && dest.exists() {
                return;
            }
        }
    }
    panic!(
        "PDFium konnte nicht heruntergeladen werden ({url}).\n\
         Bitte einmalig manuell ausführen:\n  curl -L -o <ziel> {url}\n\
         und dann DRAWGRAB_PDFIUM_LIB=<pfad zur Bibliothek> setzen."
    );
}

/// Entpackt die Bibliothek aus dem Archiv und gibt ihren Pfad zurück.
fn extract_library(archive: &Path, out_dir: &Path) -> Option<PathBuf> {
    let file = fs::File::open(archive).ok()?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(decoder);

    let wanted: [&str; 3] = ["libpdfium.dylib", "libpdfium.so", "pdfium.dll"];

    for entry in tar.entries().ok()? {
        let mut entry = entry.ok()?;
        let path = entry.path().ok()?.to_path_buf();
        let name = path.file_name()?.to_string_lossy().to_string();
        if !wanted.contains(&name.as_str()) {
            continue;
        }
        let target = out_dir.join(&name);
        let mut buffer = Vec::new();
        entry.read_to_end(&mut buffer).ok()?;
        fs::write(&target, buffer).ok()?;
        return Some(target);
    }
    None
}

/// Erzeugt die Rust-Datei, die die Bibliothek einbettet.
fn emit_library(library: &Path, out_dir: &Path) {
    let path = library.to_string_lossy();
    let generated = format!(
        "pub static PDFIUM_BYTES: &[u8] = include_bytes!(r#\"{}\"#);\n",
        path
    );
    let file = out_dir.join("pdfium_bin.rs");
    fs::write(&file, generated).expect("pdfium_bin.rs konnte nicht erzeugt werden");
}
