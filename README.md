# DrawGrab 📐

> **Automatisierte Schriftfeld-Erkennung für technische Zeichnungen (PDF) mit CSV-Export.**

DrawGrab liest automatisch relevante Metadaten aus den Schriftfeldern technischer Zeichnungen (PDF-Format) aus, stellt sie in einer interaktiven Tabelle übersichtlich dar und ermöglicht den direkten Export als Excel-kompatible CSV-Datei.

---

## ✨ Features

- **Drag & Drop:** Einfach einzelne PDF-Dateien oder ganze Ordnerstrukturen direkt in die Anwendung ziehen.
- **Automatische Felderkennung:**
  - 📄 **Zeichnungsnummer**
  - 🏷️ **Bezeichnung**
  - 🔩 **Material / Werkstoff**
  - 🛡️ **Oberflächenbehandlung / Wärmebehandlung**
- **Interaktive Tabellenansicht (egui):**
  - **Frei verstellbare Spaltenbreiten:** Spaltentrenner können beliebig mit der Maus verschoben werden.
  - **Horizontales & vertikales Scrollen:** Keine abgeschnittenen Texte bei langen Beschreibungen.
  - **Direkte Nachbearbeitung:** Alle Werte können vor dem Export direkt in der Tabelle korrigiert oder ergänzt werden.
  - **Detail-Tooltips:** Hover über Zellen zeigt vollständige Dateipfade, lange Texte und Statusmeldungen im Volltext.
  - **Spaltenbreiten-Reset:** Ein Klick auf `⟲ Spaltenbreiten` stellt die optimierten Standardbreiten wieder her.
- **Excel-optimierter CSV-Export:**
  - Export mit Semikolon (`;`) als Trennzeichen.
  - Automatisches UTF-8 BOM, damit Umlaute und Sonderzeichen direkt in Microsoft Excel korrekt geöffnet werden.
- **Integrierte PDFium-Engine:**
  - Schnelles und präzises Auslesen von Texten und Vektorelementen ohne externe Abhängigkeiten (PDFium wird beim Build automatisch plattformspezifisch eingebettet).
- **Zwei Betriebsmodi:**
  - **GUI-Modus:** Komfortable grafische Desktop-Oberfläche.
  - **CLI-Modus:** Schnelle Stapelverarbeitung für Skripte und Terminals (`--cli`).

---

## 🚀 Schnellstart

### Voraussetzungen

- [Rust & Cargo](https://rustup.rs/) (Edition 2021)
- Internetverbindung beim ersten Build (lädt die passende PDFium-Binärdatei herunter)

### Kompilieren und Starten

```bash
# Repository klonen
git clone https://github.com/hannesfox/DrawGrab.git
cd DrawGrab

# Anwendung im Release-Modus starten (GUI)
cargo run --release
```

---

## 🖥️ Bedienung

1. **Dateien hinzufügen:**
   - PDFs oder Ordner per **Drag & Drop** in das Anwendungsfenster ziehen, oder
   - Die Buttons **`📄 Dateien wählen …`** bzw. **`📁 Ordner wählen …`** nutzen.
2. **Auswertung starten:**
   - Auf **`▶ Alle auswerten`** klicken. Die Erkennung läuft asynchron im Hintergrund, während die UI flüssig bleibt.
3. **Ergebnisse prüfen & anpassen:**
   - In der Tabelle werden Status und extrahierte Felder angezeigt (grün = vollständig, gelb = Felder fehlen, rot = Fehler).
   - Bei Bedarf Werte direkt im Textfeld korrigieren.
4. **Exportieren:**
   - Mit **`⬇ CSV exportieren`** als `.csv` speichern.

---

## 💻 Kommandozeile (CLI)

DrawGrab kann auch ohne grafische Oberfläche zur automatisierten Stapelverarbeitung ausgeführt werden:

```bash
# Einen oder mehrere Ordner rekursiv auswerten
cargo run --release -- --cli /Pfad/zu/Zeichnungen

# Debug-Dump der Textelemente und Koordinaten einer Zeichnung
cargo run --release -- --debug /Pfad/zu/zeichnung.pdf
```

---

## 🛠️ Technologie-Stack

- **Programmiersprache:** [Rust](https://www.rust-lang.org/)
- **GUI Framework:** [egui](https://github.com/emilk/egui) / [eframe](https://github.com/emilk/egui/tree/master/crates/eframe) & [egui_extras](https://github.com/emilk/egui/tree/master/crates/egui_extras)
- **PDF-Rendering & Textextraktion:** [pdfium-render](https://crates.io/crates/pdfium-render) (Google PDFium)
- **Dateidialoge:** [rfd](https://crates.io/crates/rfd) (Native File Dialogs)
- **CSV-Generierung:** [csv](https://crates.io/crates/csv)

---

## 📄 Lizenz

Dieses Projekt ist unter der **Apache 2.0 Lizenz** lizenziert – siehe [LICENSE](LICENSE) für Details.
