# DrawGrab 📐

> **Automatisierte Schriftfeld-Erkennung für technische Zeichnungen (PDF) mit interaktivem Teach-In und CSV-Export.**

DrawGrab liest automatisch relevante Metadaten aus den Schriftfeldern technischer Zeichnungen (PDF-Format) aus, stellt sie in einer interaktiven Tabelle übersichtlich dar und ermöglicht den direkten Export als Excel-kompatible CSV-Datei. Über den integrierten **visuellen Teach-In-Modus** können beliebig viele unterschiedliche Zeichnungslayouts intuitiv per Mausklick angelernt werden.

---

## ✨ Features

- **Drag & Drop:** Einfach einzelne PDF-Dateien oder ganze Ordnerstrukturen direkt in die Anwendung ziehen.
- **🎓 Interaktiver Teach-In-Modus (Vorlagen anlernen):**
  - Unterstützt unterschiedliche Zeichnungslayouts von verschiedenen Kunden, Lieferanten oder CAD-Systemen.
  - Visueller Editor: Beispiel-Zeichnung laden, hineinzoomen und die Schriftfelder einfach mit der Maus markieren.
  - **Live-Textextraktion:** Der erkannte Text wird sofort beim Loslassen der Maustaste zur Überprüfung angezeigt.
  - Vorlagen werden dauerhaft in `templates.json` gespeichert und können automatisch erkannt oder manuell ausgewählt werden.
- **Automatische Felderkennung (mit heuristischem Fallback):**
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

### 1. Zeichnungen auswerten
1. **Dateien hinzufügen:** PDFs oder Ordner per **Drag & Drop** in das Anwendungsfenster ziehen (oder Buttons nutzen).
2. **Erkennungsmodus wählen:**
   - `🔄 Automatisch erkennen` (Standard): Nutzt gelernte Vorlagen, falls vorhanden; sonst die integrierte Raster-Heuristik.
   - Spezifische Vorlage auswählen oder `⚙️ Nur Standard-Heuristik`.
3. **Auswerten & Exportieren:**
   - Auf **`▶ Alle auswerten`** klicken.
   - Werte in der Tabelle prüfen und ggf. direkt anpassen.
   - Mit **`⬇ CSV exportieren`** als `.csv` speichern.

### 2. Neue Zeichnungsvorlage anlernen (Teach-In)
1. In der Werkzeugleiste auf **`🎓 Vorlage anlernen …`** klicken.
2. Im Teach-In-Fenster auf **`📄 Beispiel-PDF wählen …`** klicken und eine Zeichnung des neuen Layouts öffnen.
3. Links das gewünschte Feld auswählen (z. B. *Zeichnungsnummer* oder *Bezeichnung*).
4. Auf dem Plan mit der **linken Maustaste** einen Rahmen um das entsprechende Feld ziehen:
   - Der Text wird sofort live ausgelesen und angezeigt.
   - Das nächste Feld wird automatisch aktiviert.
5. Navigation im Plan:
   - **Rechtsklick & Ziehen:** Plan verschieben (Pan).
   - **Mausrad:** Hinein- und Herauszoomen.
6. Namen eingeben (z. B. *„Kunde ABC – A3“*) und auf **`💾 Vorlage speichern & aktivieren`** klicken.
7. Die Vorlage ist ab sofort dauerhaft gespeichert und wird bei zukünftigen Auswertungen automatisch erkannt!

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
- **Serialisierung:** [serde](https://crates.io/crates/serde) & [serde_json](https://crates.io/crates/serde_json)
- **Dateidialoge:** [rfd](https://crates.io/crates/rfd) (Native File Dialogs)
- **CSV-Generierung:** [csv](https://crates.io/crates/csv)

---

## 📄 Lizenz

Dieses Projekt ist unter der **Apache 2.0 Lizenz** lizenziert – siehe [LICENSE](LICENSE) für Details.
