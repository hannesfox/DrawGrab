//! Grafische Oberfläche: Dateien per Drag & Drop sammeln, auswerten, Tabelle prüfen,
//! als Excel exportieren sowie interaktives Anlernen neuer Vorlagen (Teach-In).

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use crate::template::{self, NormalizedRect, Template};
use crate::{backend, pdfium_loader};
use eframe::egui;

/// Eine Zeile der Ergebnistabelle.
struct Row {
    file_name: String,
    path: PathBuf,
    nummer: String,
    bezeichnung: String,
    material: String,
    oberflaeche: String,
    status: Status,
}

enum Status {
    Wartend,
    Ok,
    Fehler(String),
    Unvollstaendig(Vec<String>),
}

impl Row {
    fn new(path: PathBuf) -> Row {
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        Row {
            file_name,
            path,
            nummer: String::new(),
            bezeichnung: String::new(),
            material: String::new(),
            oberflaeche: String::new(),
            status: Status::Wartend,
        }
    }
}

enum Command {
    Process(Vec<PathBuf>, Option<String>, Vec<Template>),
    RenderPreview(PathBuf),
    /// PDF-Vorschau-Tab: Seite rendern + erkannte Bereiche bestimmen.
    RenderPdfVorschau(PathBuf, i32),
}

enum Event {
    Done(PathBuf, Vec<(String, String)>, Vec<String>),
    Failed(PathBuf, String),
    PreviewReady(
        PathBuf,
        Result<(u32, u32, Vec<u8>, backend::PageData), String>,
    ),
    PdfVorschauReady(
        PathBuf,
        Result<(u32, u32, Vec<u8>, backend::PageData), String>,
    ),
}

/// Ausgewählter Reiter der Oberfläche.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    /// Seite 1: PDFs reinziehen, auswerten, Ergebnisse, Excel.
    Auswertung,
    /// PDF ansehen + gefundene Bereiche eingerahmt.
    PdfVorschau,
    /// Neue Vorlage anlernen (Teach-In).
    VorlageAnlernen,
}

/// Zustand des PDF-Vorschau-Tabs.
struct PreviewState {
    pdf_path: Option<PathBuf>,
    page_index: i32,
    texture: Option<egui::TextureHandle>,
    page_data: Option<backend::PageData>,
    image_width: f32,
    image_height: f32,
    loading: bool,
    status_msg: String,
    /// Auf der Seite gefundene Bereiche (eingerahmt).
    regions: Vec<template::FoundRegion>,
    zoom: f32,
    pan: egui::Vec2,
    /// Zeichnung automatisch an Fenstergröße anpassen.
    autofit: bool,
}

impl Default for PreviewState {
    fn default() -> Self {
        Self {
            pdf_path: None,
            page_index: 0,
            texture: None,
            page_data: None,
            image_width: 800.0,
            image_height: 600.0,
            loading: false,
            status_msg: "Zeichnung links auswählen, um die gefundenen Bereiche zu sehen."
                .to_string(),
            regions: Vec::new(),
            zoom: 0.55,
            pan: egui::Vec2::new(20.0, 20.0),
            autofit: true,
        }
    }
}

pub fn run() {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 780.0])
            .with_min_inner_size([900.0, 500.0])
            .with_title("DrawGrab 1.0 – Schriftfelder aus Zeichnungen lesen"),
        ..Default::default()
    };

    eframe::run_native(
        "DrawGrab",
        native_options,
        Box::new(|_cc| Ok(Box::new(DrawGrabApp::new()))),
    )
    .expect("Fenster konnte nicht geöffnet werden");
}

#[derive(Clone)]
struct TeachInField {
    key: &'static str,
    label: &'static str,
    color: egui::Color32,
    rect: Option<NormalizedRect>,
    preview_text: String,
}

struct TeachInState {
    pdf_path: Option<PathBuf>,
    texture: Option<egui::TextureHandle>,
    page_data: Option<backend::PageData>,
    image_width: f32,
    image_height: f32,
    active_field_index: usize,
    fields: [TeachInField; 4],
    drag_start: Option<egui::Pos2>,
    drag_current: Option<egui::Pos2>,
    template_name: String,
    zoom: f32,
    pan: egui::Vec2,
    status_msg: String,
    loading: bool,
    /// Angeforderter Einpass-Bereich (wird im Canvas einmalig angewandt).
    fit_request: Option<NormalizedRect>,
}

impl Default for TeachInState {
    fn default() -> Self {
        Self {
            pdf_path: None,
            texture: None,
            page_data: None,
            image_width: 800.0,
            image_height: 600.0,
            active_field_index: 0,
            fields: [
                TeachInField {
                    key: "zeichnung",
                    label: "Zeichnungsnummer",
                    color: egui::Color32::from_rgb(33, 150, 243), // Blau
                    rect: None,
                    preview_text: String::new(),
                },
                TeachInField {
                    key: "bezeichnung",
                    label: "Bezeichnung",
                    color: egui::Color32::from_rgb(76, 175, 80), // Grün
                    rect: None,
                    preview_text: String::new(),
                },
                TeachInField {
                    key: "material",
                    label: "Material",
                    color: egui::Color32::from_rgb(255, 152, 0), // Orange
                    rect: None,
                    preview_text: String::new(),
                },
                TeachInField {
                    key: "oberflaeche",
                    label: "Oberflächenbehandlung",
                    color: egui::Color32::from_rgb(156, 39, 176), // Lila
                    rect: None,
                    preview_text: String::new(),
                },
            ],
            drag_start: None,
            drag_current: None,
            template_name: String::new(),
            zoom: 0.6,
            pan: egui::Vec2::new(20.0, 20.0),
            status_msg: "Wähle eine Beispiel-PDF-Zeichnung aus, um zu beginnen.".to_string(),
            loading: false,
            fit_request: None,
        }
    }
}

struct DrawGrabApp {
    rows: Vec<Row>,
    /// Dateien, die hinzugefügt wurden, aber noch nicht ausgewertet sind
    neu: Vec<PathBuf>,
    tx: Sender<Command>,
    rx: Receiver<Event>,
    status_text: String,
    reset_table_columns: bool,
    templates: Vec<Template>,
    selected_template_id: Option<String>,
    teach_in: TeachInState,
    tab: Tab,
    preview: PreviewState,
}

impl DrawGrabApp {
    fn new() -> DrawGrabApp {
        let (tx_cmd, rx_cmd) = mpsc::channel::<Command>();
        let (tx_event, rx_event) = mpsc::channel::<Event>();

        // Hintergrund-Thread: PDFium ist threadsicher (Feature "thread_safe").
        thread::Builder::new()
            .name("drawgrab-worker".to_string())
            .spawn(move || {
                let pdfium = match pdfium_loader::load_pdfium() {
                    Ok(p) => p,
                    Err(e) => {
                        let _ = tx_event.send(Event::Failed(
                            PathBuf::new(),
                            format!("PDFium konnte nicht geladen werden: {e}"),
                        ));
                        return;
                    }
                };

                while let Ok(command) = rx_cmd.recv() {
                    match command {
                        Command::Process(files, template_id, templates) => {
                            for file in files {
                                match backend::load_pages(&pdfium, &file) {
                                    Ok(pages) => {
                                        let result = template::extract_with_mode(
                                            &pages,
                                            &templates,
                                            template_id.as_deref(),
                                        );
                                        if tx_event
                                            .send(Event::Done(
                                                file.clone(),
                                                result.values,
                                                result.missing,
                                            ))
                                            .is_err()
                                        {
                                            return;
                                        }
                                    }
                                    Err(e) => {
                                        if tx_event.send(Event::Failed(file.clone(), e)).is_err() {
                                            return;
                                        }
                                    }
                                }
                            }
                        }
                        Command::RenderPreview(file) => {
                            let result = backend::render_page_preview(&pdfium, &file, 0, 1600);
                            let _ = tx_event.send(Event::PreviewReady(file, result));
                        }
                        Command::RenderPdfVorschau(file, page_index) => {
                            let result =
                                backend::render_page_preview(&pdfium, &file, page_index, 1600);
                            let _ = tx_event.send(Event::PdfVorschauReady(file, result));
                        }
                    }
                }
            })
            .expect("Hintergrund-Thread konnte nicht gestartet werden");

        let templates = template::load_templates();

        DrawGrabApp {
            rows: Vec::new(),
            neu: Vec::new(),
            tx: tx_cmd,
            rx: rx_event,
            status_text: "Bereit – ziehe Zeichnungen (PDF) hierher.".to_string(),
            reset_table_columns: false,
            templates,
            selected_template_id: None,
            teach_in: TeachInState::default(),
            tab: Tab::Auswertung,
            preview: PreviewState::default(),
        }
    }

    fn add_paths(&mut self, paths: &[PathBuf]) {
        let mut added = 0usize;
        for path in paths {
            let mut found: Vec<PathBuf> = Vec::new();
            collect_pdfs(path, &mut found);
            for file in found {
                if self.rows.iter().any(|r| r.path == file) {
                    continue;
                }
                self.rows.push(Row::new(file.clone()));
                self.neu.push(file);
                added += 1;
            }
        }
        if added > 0 {
            self.status_text = format!("{added} Datei(en) hinzugefügt – „Alle auswerten“ starten.");
        }
    }

    fn start_processing(&mut self) {
        if self.neu.is_empty() {
            self.status_text = "Keine neuen Dateien – bitte zuerst Zeichnungen hinzufügen.".into();
            return;
        }
        let files = std::mem::take(&mut self.neu);
        let count = files.len();
        for row in self.rows.iter_mut() {
            if files.contains(&row.path) {
                row.status = Status::Wartend;
            }
        }
        if self
            .tx
            .send(Command::Process(
                files,
                self.selected_template_id.clone(),
                self.templates.clone(),
            ))
            .is_ok()
        {
            let tmpl_info = match &self.selected_template_id {
                None => "automatische Vorlagenerkennung".to_string(),
                Some(id) if id == "none" => "Standard-Heuristik".to_string(),
                Some(id) => self
                    .templates
                    .iter()
                    .find(|t| &t.id == id)
                    .map(|t| format!("Vorlage „{}“", t.name))
                    .unwrap_or_else(|| "Vorlage".to_string()),
            };
            self.status_text = format!("{count} Datei(en) werden ausgewertet ({tmpl_info}) …");
        }
    }

    /// Holt Ergebnisse aus dem Hintergrund-Thread. Gibt die Anzahl neuer Ereignisse zurück.
    fn poll_events(&mut self, ctx: &egui::Context) -> usize {
        let mut processed = 0usize;
        while let Ok(event) = self.rx.try_recv() {
            match event {
                Event::Done(path, values, missing) => {
                    if let Some(row) = self.rows.iter_mut().find(|r| r.path == path) {
                        for (key, value) in values {
                            match key.as_str() {
                                "zeichnung" => row.nummer = value,
                                "bezeichnung" => {
                                    row.bezeichnung = crate::extract::normalize_designation(&value)
                                }
                                "material" => {
                                    row.material = crate::extract::normalize_material(&value)
                                }
                                "oberflaeche" => row.oberflaeche = value,
                                _ => {}
                            }
                        }
                        row.status = if missing.is_empty() {
                            Status::Ok
                        } else {
                            Status::Unvollstaendig(missing)
                        };
                    }
                    processed += 1;
                }
                Event::Failed(path, message) => {
                    if let Some(row) = self.rows.iter_mut().find(|r| r.path == path) {
                        row.status = Status::Fehler(message);
                    }
                    processed += 1;
                }
                Event::PreviewReady(path, result) => {
                    self.teach_in.loading = false;
                    match result {
                        Ok((w, h, rgba, pdata)) => {
                            let color_image = egui::ColorImage::from_rgba_unmultiplied(
                                [w as usize, h as usize],
                                &rgba,
                            );
                            self.teach_in.texture = Some(ctx.load_texture(
                                "teach_in_preview",
                                color_image,
                                egui::TextureOptions::LINEAR,
                            ));
                            self.teach_in.image_width = w as f32;
                            self.teach_in.image_height = h as f32;
                            self.teach_in.page_data = Some(pdata);
                            self.teach_in.status_msg = format!(
                                "Zeichnung geladen: {}",
                                path.file_name().unwrap_or_default().to_string_lossy()
                            );
                            self.teach_in.zoom = 0.55;
                            self.teach_in.pan = egui::Vec2::new(20.0, 20.0);

                            // Wenn Felder bereits markiert waren, Text neu extrahieren
                            if let Some(ref pd) = self.teach_in.page_data {
                                for f in self.teach_in.fields.iter_mut() {
                                    if let Some(r) = f.rect {
                                        f.preview_text = template::extract_field_text(pd, &r);
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            self.teach_in.status_msg = format!("Fehler beim Laden: {e}");
                        }
                    }
                    processed += 1;
                }
                Event::PdfVorschauReady(path, result) => {
                    self.preview.loading = false;
                    match result {
                        Ok((w, h, rgba, pdata)) => {
                            let color_image = egui::ColorImage::from_rgba_unmultiplied(
                                [w as usize, h as usize],
                                &rgba,
                            );
                            self.preview.texture = Some(ctx.load_texture(
                                "pdf_vorschau_preview",
                                color_image,
                                egui::TextureOptions::LINEAR,
                            ));
                            self.preview.image_width = w as f32;
                            self.preview.image_height = h as f32;
                            self.preview.regions = template::found_regions(
                                &pdata,
                                &self.templates,
                                self.selected_template_id.as_deref(),
                            );
                            self.preview.page_data = Some(pdata);
                            self.preview.status_msg = format!(
                                "Seite {} geladen: {}",
                                self.preview.page_index + 1,
                                path.file_name().unwrap_or_default().to_string_lossy()
                            );
                        }
                        Err(e) => {
                            self.preview.status_msg = format!("Fehler beim Laden: {e}");
                            self.preview.texture = None;
                            self.preview.regions.clear();
                            // Seiten-Überlauf (zu hoher Seitenindex) zurücksetzen
                            if self.preview.page_index > 0 {
                                self.preview.page_index -= 1;
                            }
                        }
                    }
                    processed += 1;
                }
            }
        }
        if processed > 0 {
            let total = self.rows.len();
            let ok = self
                .rows
                .iter()
                .filter(|r| matches!(r.status, Status::Ok))
                .count();
            let unvoll = self
                .rows
                .iter()
                .filter(|r| matches!(r.status, Status::Unvollstaendig(_)))
                .count();
            let fehler = self
                .rows
                .iter()
                .filter(|r| matches!(r.status, Status::Fehler(_)))
                .count();
            self.status_text = format!(
                "{processed} Datei(en) ausgewertet – {ok} vollständig, {unvoll} mit Lücken, {fehler} Fehler (gesamt {total})."
            );
        }
        processed
    }

    /// Exportiert die Ergebnisliste als Excel-Datei (.xlsx).
    fn export_excel(&mut self) -> Result<(), String> {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Excel exportieren")
            .add_filter("Excel-Datei", &["xlsx"])
            .set_file_name("zeichnungen.xlsx")
            .save_file()
        else {
            return Ok(());
        };

        let mut workbook = rust_xlsxwriter::Workbook::new();
        let worksheet = workbook
            .add_worksheet()
            .set_name("Zeichnungen")
            .map_err(|e| format!("Excel-Workbook konnte nicht erstellt werden: {e}"))?;

        // Spaltenstruktur entspricht der Lieferantenfanfrage (VK-Import-Master).
        let headers: [&str; 12] = [
            "Positionsnummer",
            "Menge",
            "Hersteller Typennummer",
            "CAD Revision",
            "Produkttext kurz (DE)",
            "Material",
            "Rohabmessungen",
            "Abmessungen",
            "Oberfläche",
            "Bemerkung (DE)",
            "Produktnummer",
            "Referenz",
        ];
        for (col, header) in headers.iter().enumerate() {
            worksheet
                .write_string(0, col as u16, *header)
                .map_err(|e| format!("Excel konnte nicht geschrieben werden: {e}"))?;
        }

        for (pos, row) in self.rows.iter().enumerate() {
            let row_idx = (pos + 1) as u32;
            let material = crate::extract::normalize_material(&row.material);
            let record: Vec<String> = vec![
                (pos + 1).to_string(), // Positionsnummer – fortlaufend
                "0".to_string(),       // Menge
                row.nummer.clone(),    // Hersteller Typennummer
                String::new(),         // CAD Revision
                crate::extract::normalize_designation(&row.bezeichnung), // Produkttext kurz (DE)
                material,              // Material (nur Werkstoffnummer)
                String::new(),         // Rohabmessungen
                String::new(),         // Abmessungen
                row.oberflaeche.clone(), // Oberfläche
                String::new(),         // Bemerkung (DE)
                String::new(),         // Produktnummer
                String::new(),         // Referenz
            ];
            worksheet
                .write_row(row_idx, 0, record)
                .map_err(|e| format!("Excel konnte nicht geschrieben werden: {e}"))?;
        }

        workbook
            .save(&path)
            .map_err(|e| format!("Excel Datei konnte nicht gespeichert werden: {e}"))?;

        self.status_text = format!("Excel exportiert: {}", path.display());
        Ok(())
    }

    /// Zeigt den Teach-In-Dialog zum interaktiven Anlernen neuer Vorlagen.
    /// Lädt die aktuell gewählte Seite des PDF-Vorschau-Tabs neu.
    fn request_preview_page(&mut self) {
        if let Some(path) = self.preview.pdf_path.clone() {
            self.preview.loading = true;
            self.preview.regions.clear();
            let _ = self
                .tx
                .send(Command::RenderPdfVorschau(path, self.preview.page_index));
        }
    }

    /// Reiter „PDF-Vorschau & erkannte Bereiche": Seite ansehen, gefundene Bereiche eingerahmt.
    fn tab_pdf_vorschau(&mut self, ui: &mut egui::Ui) {
        ui.heading("🔍 PDF-Vorschau & erkannte Bereiche");
        ui.add_space(4.0);

        // --- Linke Seite: Liste der importierten PDFs -----------------------
        let sel_path = self.preview.pdf_path.clone();
        let paths: Vec<PathBuf> = self.rows.iter().map(|r| r.path.clone()).collect();
        egui::Panel::left("vorschau_liste")
            .resizable(true)
            .default_size(240.0)
            .size_range(140.0..=440.0)
            .show_inside(ui, |ui| {
                ui.heading("Zeichnungen");
                ui.label(format!("{} importiert", paths.len()));
                ui.separator();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if paths.is_empty() {
                            ui.label("Noch keine PDFs. Auf Reiter „📋 Auswertung“ hinzufügen.");
                        }
                        for p in &paths {
                            let name = p
                                .file_name()
                                .map(|s| s.to_string_lossy().to_string())
                                .unwrap_or_default();
                            let selected = sel_path.as_deref() == Some(p.as_path());
                            if ui.selectable_label(selected, name).clicked() && !selected {
                                self.preview.pdf_path = Some(p.clone());
                                self.preview.page_index = 0;
                                self.preview.loading = true;
                                self.preview.regions.clear();
                                self.preview.autofit = true;
                                let _ = self.tx.send(Command::RenderPdfVorschau(p.clone(), 0));
                            }
                        }
                    });
            });

        // --- Rechte Seite: Steuerung + Canvas -------------------------------
        ui.horizontal(|ui| {
            let has = self.preview.pdf_path.is_some();
            if ui
                .add_enabled(
                    has && self.preview.page_index > 0,
                    egui::Button::new("◀ Seite"),
                )
                .clicked()
            {
                self.preview.page_index -= 1;
                self.request_preview_page();
            }
            ui.label(format!("Seite {}", self.preview.page_index + 1));
            if ui.add_enabled(has, egui::Button::new("Seite ▶")).clicked() {
                self.preview.page_index += 1;
                self.request_preview_page();
            }

            ui.separator();
            if ui.button("➖").clicked() {
                self.preview.autofit = false;
                self.preview.zoom = (self.preview.zoom * 0.85).max(0.15);
            }
            if ui.button("100%").clicked() {
                self.preview.autofit = false;
                self.preview.zoom = 1.0;
                self.preview.pan = egui::Vec2::ZERO;
            }
            if ui.button("➕").clicked() {
                self.preview.autofit = false;
                self.preview.zoom = (self.preview.zoom * 1.15).min(8.0);
            }
            if ui
                .button("⟲ Ansicht einpassen")
                .on_hover_text("Zeichnung an Fenstergröße anpassen")
                .clicked()
            {
                self.preview.autofit = true;
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(format!("{} Bereich(e) erkannt", self.preview.regions.len()));
            });
        });

        ui.label(&self.preview.status_msg);
        ui.add_space(4.0);

        let canvas_size = ui.available_size();
        let (response, painter) = ui.allocate_painter(canvas_size, egui::Sense::click_and_drag());
        let canvas_rect = response.rect;

        // Zoom per Mausrad (deaktiviert Auto-Einpassen)
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                let factor = if scroll > 0.0 { 1.1 } else { 0.9 };
                self.preview.autofit = false;
                self.preview.zoom = (self.preview.zoom * factor).clamp(0.15, 8.0);
            }
        }
        // Pan per Rechtsklick/Mittelklick
        let is_panning = ui.input(|i| {
            i.pointer.button_down(egui::PointerButton::Secondary)
                || i.pointer.button_down(egui::PointerButton::Middle)
        });
        if response.dragged() && is_panning {
            self.preview.autofit = false;
            self.preview.pan += response.drag_delta();
        }

        let nw = self.preview.image_width.max(1.0);
        let nh = self.preview.image_height.max(1.0);
        // Zeichnung an Fenstergröße anpassen, solange Auto-Einpassen aktiv ist.
        let (zoom, pan) = if self.preview.autofit {
            let (z, pn) = fit_normalized_rect(
                canvas_rect,
                nw,
                nh,
                NormalizedRect::new(0.0, 0.0, 1.0, 1.0),
                0.02,
            );
            self.preview.zoom = z;
            self.preview.pan = pn;
            (z, pn)
        } else {
            (self.preview.zoom, self.preview.pan)
        };

        let img_w = nw * zoom;
        let img_h = nh * zoom;
        let img_rect = egui::Rect::from_min_size(canvas_rect.min + pan, egui::vec2(img_w, img_h));

        painter.rect_filled(canvas_rect, 4.0, egui::Color32::from_rgb(28, 30, 34));

        if let Some(ref texture) = self.preview.texture {
            painter.image(
                texture.id(),
                img_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
            painter.rect_stroke(
                img_rect,
                0.0,
                egui::Stroke::new(1.0, egui::Color32::GRAY),
                egui::StrokeKind::Outside,
            );
        } else {
            let msg = if self.preview.loading {
                "PDF wird geladen & gerendert …"
            } else if self.preview.pdf_path.is_none() {
                "Links eine Zeichnung auswählen"
            } else {
                "Keine Vorschau"
            };
            painter.text(
                canvas_rect.center(),
                egui::Align2::CENTER_CENTER,
                msg,
                egui::FontId::proportional(16.0),
                egui::Color32::LIGHT_GRAY,
            );
        }

        // --- Gefundene Bereiche einrahmen -----------------------------------
        for region in &self.preview.regions {
            let color = region_color(&region.key);
            let min = img_rect.min
                + egui::vec2(
                    region.rect.x0 * img_rect.width(),
                    region.rect.y0 * img_rect.height(),
                );
            let max = img_rect.min
                + egui::vec2(
                    region.rect.x1 * img_rect.width(),
                    region.rect.y1 * img_rect.height(),
                );
            let r = egui::Rect::from_min_max(min, max);
            let mut fill = color;
            fill = egui::Color32::from_rgba_unmultiplied(fill.r(), fill.g(), fill.b(), 45);
            painter.rect_filled(r, 2.0, fill);
            painter.rect_stroke(
                r,
                2.0,
                egui::Stroke::new(2.0, color),
                egui::StrokeKind::Outside,
            );

            let snippet: String = region.value.chars().take(22).collect();
            let badge_text = format!("{}: \"{}\"", region.label, snippet);
            let badge_pos = r.left_top() - egui::vec2(0.0, 18.0);
            painter.rect_filled(
                egui::Rect::from_min_size(
                    badge_pos,
                    egui::vec2(badge_text.chars().count() as f32 * 7.0 + 8.0, 18.0),
                ),
                3.0,
                color,
            );
            painter.text(
                badge_pos + egui::vec2(4.0, 2.0),
                egui::Align2::LEFT_TOP,
                badge_text,
                egui::FontId::proportional(11.0),
                egui::Color32::WHITE,
            );
        }

        ui.small("💡 Rechtsklick = Plan verschieben | Mausrad = Zoomen | farbiger Rahmen = erkannter Bereich");
    }

    /// Reiter „Vorlage anlernen" (Teach-In): neue Vorlage per Mausauswahl anlernen.
    fn tab_vorlage_anlernen(&mut self, ui: &mut egui::Ui) {
        ui.heading("🎓 Vorlage anlernen");
        ui.label("Beispiel-PDF wählen und die zu findenden Bereiche markieren.");
        ui.add_space(4.0);
        {
            ui.horizontal(|ui| {
                if ui.button("📄 Beispiel-PDF wählen …").clicked() {
                    if let Some(file) = rfd::FileDialog::new()
                        .set_title("Beispiel-Zeichnung (PDF) für Vorlage wählen")
                        .add_filter("Zeichnungen (PDF)", &["pdf"])
                        .pick_file()
                    {
                        self.teach_in.pdf_path = Some(file.clone());
                        self.teach_in.loading = true;
                        self.teach_in.status_msg = format!(
                            "Lade {} …",
                            file.file_name().unwrap_or_default().to_string_lossy()
                        );
                        if self.teach_in.template_name.is_empty() {
                            self.teach_in.template_name = file
                                .file_stem()
                                .map(|s| s.to_string_lossy().to_string())
                                .unwrap_or_else(|| "Neue Vorlage".to_string());
                        }
                        let _ = self.tx.send(Command::RenderPreview(file));
                    }
                }

                if let Some(ref path) = self.teach_in.pdf_path {
                    ui.label(format!(
                        "Geladen: {}",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    ));
                } else {
                    ui.label("Keine PDF geladen");
                }

                ui.separator();
                ui.label(&self.teach_in.status_msg);
            });

            ui.separator();

            egui::Panel::left("teach_in_left")
                .resizable(true)
                .default_size(240.0)
                .size_range(150.0..=320.0)
                .show_inside(ui, |left| {
                    // --- Linke Spalte: Steuerleiste & Felder (kompakt) ---

                    left.heading("1. Schriftfeld markieren");
                    left.label(
                        "Wähle ein Feld aus und ziehe mit der Maus im rechten Plan ein Rechteck darum auf:",
                    );
                    left.add_space(4.0);

                    let mut remove_field_idx = None;
                    let mut select_field_idx = None;

                    for i in 0..4 {
                        let is_active = self.teach_in.active_field_index == i;
                        let label = self.teach_in.fields[i].label;
                        let field_color = self.teach_in.fields[i].color;
                        let has_rect = self.teach_in.fields[i].rect.is_some();
                        let rect_opt = self.teach_in.fields[i].rect;
                        let preview_text = self.teach_in.fields[i].preview_text.clone();

                        left.horizontal(|ui| {
                            let text = if is_active {
                                format!("👉 {label}")
                            } else {
                                label.to_string()
                            };

                            let btn = egui::Button::new(
                                egui::RichText::new(text).color(if is_active {
                                    egui::Color32::WHITE
                                } else {
                                    field_color
                                }),
                            )
                            .fill(if is_active {
                                field_color
                            } else {
                                ui.visuals().widgets.inactive.bg_fill
                            });

                            if ui.add(btn).clicked() {
                                select_field_idx = Some(i);
                            }

                            if has_rect {
                                ui.colored_label(egui::Color32::from_rgb(0x4c, 0xaf, 0x50), "✔");
                                if ui.small_button("✕").on_hover_text("Markierung entfernen").clicked() {
                                    remove_field_idx = Some(i);
                                }
                            } else {
                                ui.colored_label(ui.visuals().weak_text_color(), "⚪");
                            }
                        });

                        // Vorschautext anzeigen
                        if let Some(r) = rect_opt {
                            left.indent(format!("preview_{i}"), |ui| {
                                let coord_info = format!(
                                    "Bereich: X {:.1}%..{:.1}%, Y {:.1}%..{:.1}%",
                                    r.x0 * 100.0,
                                    r.x1 * 100.0,
                                    r.y0 * 100.0,
                                    r.y1 * 100.0
                                );
                                ui.small(coord_info);
                                if preview_text.is_empty() {
                                    ui.colored_label(
                                        egui::Color32::from_rgb(0xb2, 0x6a, 0x00),
                                        "Kein Text im Bereich erkannt",
                                    );
                                } else {
                                    ui.monospace(format!("„{preview_text}“"));
                                }
                            });
                        }
                        left.add_space(3.0);
                    }

                    if let Some(idx) = select_field_idx {
                        self.teach_in.active_field_index = idx;
                    }
                    if let Some(idx) = remove_field_idx {
                        self.teach_in.fields[idx].rect = None;
                        self.teach_in.fields[idx].preview_text.clear();
                    }

                    left.separator();
                    left.heading("2. Vorlage speichern");
                    left.horizontal(|ui| {
                        ui.label("Name:");
                        ui.text_edit_singleline(&mut self.teach_in.template_name);
                    });

                    left.add_space(4.0);
                    if left
                        .button(egui::RichText::new("💾 Vorlage speichern & aktivieren").strong())
                        .clicked()
                    {
                        if self.teach_in.template_name.trim().is_empty() {
                            self.teach_in.status_msg =
                                "Bitte einen Namen für die Vorlage vergeben!".to_string();
                        } else if self.teach_in.fields.iter().all(|f| f.rect.is_none()) {
                            self.teach_in.status_msg =
                                "Bitte mindestens ein Schriftfeld auf dem Plan markieren!"
                                    .to_string();
                        } else {
                            let mut tmpl = Template::new(&self.teach_in.template_name);
                            for f in &self.teach_in.fields {
                                if let Some(r) = f.rect {
                                    tmpl.set_field(f.key, f.label, r);
                                }
                            }
                            let tmpl_id = tmpl.id.clone();
                            let tmpl_name = tmpl.name.clone();

                            if let Some(existing) =
                                self.templates.iter_mut().find(|t| t.name == tmpl.name)
                            {
                                *existing = tmpl;
                            } else {
                                self.templates.push(tmpl);
                            }

                            if let Err(e) = template::save_templates(&self.templates) {
                                self.teach_in.status_msg =
                                    format!("Fehler beim Speichern: {e}");
                            } else {
                                self.selected_template_id = Some(tmpl_id);
                                self.teach_in.status_msg =
                                    format!("Vorlage „{tmpl_name}“ erfolgreich gespeichert!");
                                self.status_text =
                                    format!("Vorlage „{tmpl_name}“ aktiv.");
                            }
                        }
                    }

                    left.separator();
                    left.collapsing(
                        format!("📁 Gespeicherte Vorlagen ({})", self.templates.len()),
                        |ui| {
                            if self.templates.is_empty() {
                                ui.label("Noch keine Vorlagen angelernt.");
                            } else {
                                let mut to_delete = None;
                                for (idx, tmpl) in self.templates.iter().enumerate() {
                                    ui.horizontal(|ui| {
                                        ui.label(format!("📐 {}", tmpl.name));
                                        if ui
                                            .button("🗑")
                                            .on_hover_text("Vorlage löschen")
                                            .clicked()
                                        {
                                            to_delete = Some(idx);
                                        }
                                    });
                                }
                                if let Some(idx) = to_delete {
                                    self.templates.remove(idx);
                                    let _ = template::save_templates(&self.templates);
                                    self.selected_template_id = None;
                                }
                            }
                        },
                    );

                    // --- Rechte Spalte: Interaktiver Canvas (Zoom / Pan / Bounding Box) ---
                });

            let right = &mut *ui;

            right.horizontal(|ui| {
                        if ui.button("➖ Zoom -").clicked() {
                            self.teach_in.zoom = (self.teach_in.zoom * 0.85).max(0.15);
                        }
                        if ui.button("100%").clicked() {
                            self.teach_in.zoom = 0.55;
                        }
                        if ui.button("➕ Zoom +").clicked() {
                            self.teach_in.zoom = (self.teach_in.zoom * 1.15).min(5.0);
                        }
                        if ui.button("⟲ Ansicht einpassen").clicked() {
                            self.teach_in.fit_request = Some(NormalizedRect::new(0.0, 0.0, 1.0, 1.0));
                        }
                        if ui
                            .button("🔍 Auf Schriftfeld zoomen")
                            .on_hover_text("Auf das Schriftfeld (markierte Bereiche, sonst rechts unten) einpassen")
                            .clicked()
                        {
                            self.teach_in.fit_request = Some(schriftfeld_rect(&self.teach_in.fields));
                        }
                        ui.separator();
                        ui.small("💡 Linksklick = Rahmen aufziehen | Rechtsklick = Plan verschieben | Mausrad = Zoomen");
                    });

            let canvas_size = right.available_size();
            let (response, painter) =
                right.allocate_painter(canvas_size, egui::Sense::click_and_drag());
            let canvas_rect = response.rect;

            // 1. Zoom per Mausrad
            if response.hovered() {
                let scroll = right.input(|i| i.smooth_scroll_delta.y);
                if scroll != 0.0 {
                    let factor = if scroll > 0.0 { 1.1 } else { 0.9 };
                    self.teach_in.zoom = (self.teach_in.zoom * factor).clamp(0.15, 6.0);
                }
            }

            // 2. Pan per Rechtsklick / Mittelklick
            let is_panning = right.input(|i| {
                i.pointer.button_down(egui::PointerButton::Secondary)
                    || i.pointer.button_down(egui::PointerButton::Middle)
            });
            if response.dragged() && is_panning {
                self.teach_in.pan += response.drag_delta();
            }

            // 3. Bildfläche berechnen
            // Angefordertes Einpassen (Ganze Seite / Schriftfeld) einmalig anwenden.
            if let Some(r) = self.teach_in.fit_request.take() {
                let nw = self.teach_in.image_width.max(1.0);
                let nh = self.teach_in.image_height.max(1.0);
                let (z, pn) = fit_normalized_rect(canvas_rect, nw, nh, r, 0.05);
                self.teach_in.zoom = z;
                self.teach_in.pan = pn;
            }
            let img_w = self.teach_in.image_width * self.teach_in.zoom;
            let img_h = self.teach_in.image_height * self.teach_in.zoom;
            let img_rect = egui::Rect::from_min_size(
                canvas_rect.min + self.teach_in.pan,
                egui::vec2(img_w, img_h),
            );

            // Hintergrund
            painter.rect_filled(canvas_rect, 4.0, egui::Color32::from_rgb(28, 30, 34));

            if let Some(ref texture) = self.teach_in.texture {
                painter.image(
                    texture.id(),
                    img_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
                painter.rect_stroke(
                    img_rect,
                    0.0,
                    egui::Stroke::new(1.0, egui::Color32::GRAY),
                    egui::StrokeKind::Outside,
                );
            } else {
                let msg = if self.teach_in.loading {
                    "PDF wird geladen & gerendert …"
                } else {
                    "Keine PDF geladen – links auf „📄 Beispiel-PDF wählen …“ klicken"
                };
                painter.text(
                    canvas_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    msg,
                    egui::FontId::proportional(16.0),
                    egui::Color32::LIGHT_GRAY,
                );
            }

            // 4. Bereits konfigurierte Felder auf dem Plan einzeichnen
            for field in &self.teach_in.fields {
                if let Some(norm_rect) = field.rect {
                    let screen_min = img_rect.min
                        + egui::vec2(
                            norm_rect.x0 * img_rect.width(),
                            norm_rect.y0 * img_rect.height(),
                        );
                    let screen_max = img_rect.min
                        + egui::vec2(
                            norm_rect.x1 * img_rect.width(),
                            norm_rect.y1 * img_rect.height(),
                        );
                    let field_screen_rect = egui::Rect::from_min_max(screen_min, screen_max);

                    let mut fill = field.color;
                    fill = egui::Color32::from_rgba_unmultiplied(fill.r(), fill.g(), fill.b(), 45);
                    painter.rect_filled(field_screen_rect, 2.0, fill);
                    painter.rect_stroke(
                        field_screen_rect,
                        2.0,
                        egui::Stroke::new(2.0, field.color),
                        egui::StrokeKind::Outside,
                    );

                    // Beschriftung über der Box
                    let badge_text = if field.preview_text.is_empty() {
                        field.label.to_string()
                    } else {
                        let snippet: String = field.preview_text.chars().take(22).collect();
                        format!("{}: \"{}\"", field.label, snippet)
                    };
                    let badge_pos = field_screen_rect.left_top() - egui::vec2(0.0, 18.0);
                    painter.rect_filled(
                        egui::Rect::from_min_size(
                            badge_pos,
                            egui::vec2(badge_text.len() as f32 * 7.0 + 8.0, 18.0),
                        ),
                        3.0,
                        field.color,
                    );
                    painter.text(
                        badge_pos + egui::vec2(4.0, 2.0),
                        egui::Align2::LEFT_TOP,
                        badge_text,
                        egui::FontId::proportional(11.0),
                        egui::Color32::WHITE,
                    );
                }
            }

            // 5. Interaktives Aufziehen mit Linksklick
            if self.teach_in.texture.is_some() && !is_panning {
                let pointer_pos = right.input(|i| i.pointer.interact_pos());
                let primary_down = right.input(|i| i.pointer.primary_down());
                let primary_clicked = response.drag_started_by(egui::PointerButton::Primary);
                let primary_released = right.input(|i| i.pointer.primary_released());

                if primary_clicked {
                    if let Some(pos) = pointer_pos {
                        if img_rect.contains(pos) {
                            self.teach_in.drag_start = Some(pos);
                            self.teach_in.drag_current = Some(pos);
                        }
                    }
                } else if primary_down && self.teach_in.drag_start.is_some() {
                    if let Some(pos) = pointer_pos {
                        self.teach_in.drag_current = Some(pos);
                    }
                }

                // Aktuelles Live-Rechteck zeichnen
                if let (Some(start), Some(curr)) =
                    (self.teach_in.drag_start, self.teach_in.drag_current)
                {
                    let drag_rect = egui::Rect::from_two_pos(start, curr);
                    let active_color = self.teach_in.fields[self.teach_in.active_field_index].color;
                    let fill = egui::Color32::from_rgba_unmultiplied(
                        active_color.r(),
                        active_color.g(),
                        active_color.b(),
                        60,
                    );
                    painter.rect_filled(drag_rect, 2.0, fill);
                    painter.rect_stroke(
                        drag_rect,
                        2.0,
                        egui::Stroke::new(2.0, active_color),
                        egui::StrokeKind::Outside,
                    );
                }

                // Beim Loslassen der Maus Feld speichern
                if primary_released {
                    if let (Some(start), Some(curr)) = (
                        self.teach_in.drag_start.take(),
                        self.teach_in.drag_current.take(),
                    ) {
                        let min_x = start.x.min(curr.x);
                        let max_x = start.x.max(curr.x);
                        let min_y = start.y.min(curr.y);
                        let max_y = start.y.max(curr.y);

                        if (max_x - min_x) > 6.0 && (max_y - min_y) > 6.0 {
                            let u0 = ((min_x - img_rect.min.x) / img_rect.width()).clamp(0.0, 1.0);
                            let v0 = ((min_y - img_rect.min.y) / img_rect.height()).clamp(0.0, 1.0);
                            let u1 = ((max_x - img_rect.min.x) / img_rect.width()).clamp(0.0, 1.0);
                            let v1 = ((max_y - img_rect.min.y) / img_rect.height()).clamp(0.0, 1.0);

                            let norm_rect = NormalizedRect::new(u0, v0, u1, v1);
                            let active_idx = self.teach_in.active_field_index;
                            self.teach_in.fields[active_idx].rect = Some(norm_rect);

                            if let Some(ref pdata) = self.teach_in.page_data {
                                let text = template::extract_field_text(pdata, &norm_rect);
                                self.teach_in.fields[active_idx].preview_text = text.clone();
                                self.teach_in.status_msg = if text.is_empty() {
                                    format!(
                                        "Feld „{}“ markiert (Kein Text im Bereich)",
                                        self.teach_in.fields[active_idx].label
                                    )
                                } else {
                                    format!(
                                        "Feld „{}“ erkannt: \"{}\"",
                                        self.teach_in.fields[active_idx].label, text
                                    )
                                };
                            }

                            // Zum nächsten noch unkonfigurierten Feld wechseln
                            if let Some(next) =
                                (0..4).find(|&idx| self.teach_in.fields[idx].rect.is_none())
                            {
                                self.teach_in.active_field_index = next;
                            }
                        }
                    }
                }
            }
        }
    }
}

impl eframe::App for DrawGrabApp {
    /// Verarbeitung ohne UI – auch wenn das Fenster gerade im Hintergrund ist.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.poll_events(ctx) > 0 {
            ctx.request_repaint();
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // Ereignisse abfragen
        if self.poll_events(&ctx) > 0 {
            ctx.request_repaint();
        }

        // Dateien per Drag & Drop aufnehmen
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        if !dropped.is_empty() {
            self.add_paths(&dropped);
        }
        let dragging = ctx.input(|i| !i.raw.hovered_files.is_empty());

        egui::Panel::top("kopf")
            .resizable(false)
            .drag_to_open(false)
            .frame(egui::Frame::default().inner_margin(12))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("DrawGrab");
                    ui.label("1.0 – Schriftfelder aus technischen Zeichnungen lesen");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(format!("{} Datei(en)", self.rows.len()));
                    });
                });
            });

        // --- Reiterleiste ----------------------------------------------------
        egui::Panel::top("tabs")
            .resizable(false)
            .drag_to_open(false)
            .frame(egui::Frame::default().inner_margin(egui::Margin::symmetric(8, 4)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.tab, Tab::Auswertung, "📋 Auswertung");
                    ui.selectable_value(
                        &mut self.tab,
                        Tab::PdfVorschau,
                        "🔍 PDF-Vorschau & erkannte Bereiche",
                    );
                    ui.selectable_value(&mut self.tab, Tab::VorlageAnlernen, "🎓 Vorlage anlernen");
                });
            });

        match self.tab {
            Tab::Auswertung => {
                egui::CentralPanel::default().show(ui, |ui| {
            // --- Ablagefläche ------------------------------------------------
            egui::Frame::group(ui.style())
                .fill(if dragging {
                    ui.visuals().selection.bg_fill
                } else {
                    ui.visuals().extreme_bg_color
                })
                .stroke(egui::Stroke::new(
                    1.5,
                    if dragging {
                        ui.visuals().selection.stroke.color
                    } else {
                        ui.visuals().widgets.noninteractive.bg_stroke.color
                    },
                ))
                .inner_margin(18)
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.vertical_centered(|ui| {
                        let text = if dragging {
                            "Dateien loslassen!"
                        } else {
                            "Zeichnungen (PDF) hierher ziehen – oder Ordner mit Zeichnungen"
                        };
                        ui.heading(text);
                        ui.label(
                            "Gelesen werden: Zeichnungsnummer, Bezeichnung, Material und Oberflächenbehandlung.",
                        );
                    });
                });

            ui.add_space(6.0);

            // --- Werkzeugleiste ---------------------------------------------
            ui.horizontal(|ui| {
                if ui.button("📄 Dateien wählen …").clicked() {
                    if let Some(files) = rfd::FileDialog::new()
                        .add_filter("Zeichnungen (PDF)", &["pdf"])
                        .pick_files()
                    {
                        self.add_paths(&files);
                    }
                }
                if ui.button("📁 Ordner wählen …").clicked() {
                    if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                        self.add_paths(&[folder]);
                    }
                }

                ui.separator();

                // Vorlagenauswahl
                egui::ComboBox::from_label("Erkennung")
                    .selected_text(match &self.selected_template_id {
                        None => "🔄 Automatisch erkennen".to_string(),
                        Some(id) if id == "none" => "⚙️ Nur Standard-Heuristik".to_string(),
                        Some(id) => self
                            .templates
                            .iter()
                            .find(|t| &t.id == id)
                            .map(|t| format!("📐 {}", t.name))
                            .unwrap_or_else(|| "Unbekannte Vorlage".to_string()),
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.selected_template_id,
                            None,
                            "🔄 Automatisch erkennen (empfohlen)",
                        );
                        ui.selectable_value(
                            &mut self.selected_template_id,
                            Some("none".to_string()),
                            "⚙️ Nur Standard-Heuristik",
                        );
                        if !self.templates.is_empty() {
                            ui.separator();
                            for tmpl in &self.templates {
                                ui.selectable_value(
                                    &mut self.selected_template_id,
                                    Some(tmpl.id.clone()),
                                    format!("📐 {}", tmpl.name),
                                );
                            }
                        }
                    });

                // Button für Teach-In Modus
                if ui
                    .button(egui::RichText::new("🎓 Vorlage anlernen …").strong())
                    .on_hover_text("Interaktives Anlernen eines neuen Schriftfeld-Layouts per Mausauswahl (öffnet den Reiter)")
                    .clicked()
                {
                    self.tab = Tab::VorlageAnlernen;
                }

                ui.separator();

                let auswertbar = !self.neu.is_empty();
                let button =
                    egui::Button::new("▶ Alle auswerten").min_size(egui::vec2(140.0, 30.0));
                if ui.add_enabled(auswertbar, button).clicked() {
                    self.start_processing();
                }

                ui.separator();

                let button =
                    egui::Button::new("⬇ Excel exportieren").min_size(egui::vec2(150.0, 30.0));
                if ui.add_enabled(!self.rows.is_empty(), button).clicked() {
                    if let Err(e) = self.export_excel() {
                        self.status_text = format!("Excel konnte nicht gespeichert werden: {e}");
                    }
                }
                if !self.rows.is_empty() {
                    if ui.button("🗑 Liste leeren").clicked() {
                        self.rows.clear();
                        self.neu.clear();
                        self.status_text = "Liste geleert.".into();
                    }
                    if ui
                        .button("⟲ Spaltenbreiten")
                        .on_hover_text("Setzt alle Spaltenbreiten auf Standardwerte zurück")
                        .clicked()
                    {
                        self.reset_table_columns = true;
                    }
                }
            });

            ui.add_space(4.0);
            ui.label(&self.status_text);
            ui.add_space(6.0);

            // --- Ergebnistabelle --------------------------------------------
            let mut rows = std::mem::take(&mut self.rows);
            let neu_flag = std::mem::take(&mut self.neu);

            egui::ScrollArea::horizontal()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let table = egui_extras::TableBuilder::new(ui)
                        .id_salt("results_table")
                        .resizable(true)
                        .column(egui_extras::Column::initial(220.0).at_least(110.0).clip(true))
                        .column(egui_extras::Column::initial(180.0).at_least(110.0).clip(true))
                        .column(egui_extras::Column::initial(320.0).at_least(150.0).clip(true))
                        .column(egui_extras::Column::initial(300.0).at_least(150.0).clip(true))
                        .column(egui_extras::Column::initial(340.0).at_least(150.0).clip(true))
                        .column(egui_extras::Column::initial(200.0).at_least(120.0).clip(true))
                        .striped(true);

                    if self.reset_table_columns {
                        table.reset();
                        self.reset_table_columns = false;
                    }

                    table
                        .header(26.0, |mut header| {
                            header.col(|ui| {
                                ui.strong("Datei");
                            });
                            header.col(|ui| {
                                ui.strong("Zeichnungsnummer");
                            });
                            header.col(|ui| {
                                ui.strong("Bezeichnung");
                            });
                            header.col(|ui| {
                                ui.strong("Material");
                            });
                            header.col(|ui| {
                                ui.strong("Oberflächenbehandlung");
                            });
                            header.col(|ui| {
                                ui.strong("Status");
                            });
                        })
                        .body(|mut body| {
                            for row in rows.iter_mut() {
                                let Row {
                                    file_name,
                                    path,
                                    nummer,
                                    bezeichnung,
                                    material,
                                    oberflaeche,
                                    status,
                                    ..
                                } = row;
                                body.row(28.0, |mut row_ui| {
                                    row_ui.col(|ui| {
                                        ui.label(file_name.as_str()).on_hover_text(format!(
                                            "Datei: {file_name}\nPfad: {}",
                                            path.display()
                                        ));
                                    });
                                    row_ui.col(|ui| {
                                        let resp = ui.add(
                                            egui::TextEdit::singleline(nummer)
                                                .desired_width(f32::INFINITY),
                                        );
                                        if !nummer.is_empty() {
                                            resp.on_hover_text(&*nummer);
                                        }
                                    });
                                    row_ui.col(|ui| {
                                        let resp = ui.add(
                                            egui::TextEdit::singleline(bezeichnung)
                                                .desired_width(f32::INFINITY),
                                        );
                                        if !bezeichnung.is_empty() {
                                            resp.on_hover_text(&*bezeichnung);
                                        }
                                    });
                                    row_ui.col(|ui| {
                                        let resp = ui.add(
                                            egui::TextEdit::singleline(material)
                                                .desired_width(f32::INFINITY),
                                        );
                                        if !material.is_empty() {
                                            resp.on_hover_text(&*material);
                                        }
                                    });
                                    row_ui.col(|ui| {
                                        let resp = ui.add(
                                            egui::TextEdit::singleline(oberflaeche)
                                                .desired_width(f32::INFINITY),
                                        );
                                        if !oberflaeche.is_empty() {
                                            resp.on_hover_text(&*oberflaeche);
                                        }
                                    });
                                    row_ui.col(|ui| match status {
                                        Status::Wartend => {
                                            ui.colored_label(
                                                ui.visuals().weak_text_color(),
                                                "wartet …",
                                            );
                                        }
                                        Status::Ok => {
                                            ui.colored_label(
                                                egui::Color32::from_rgb(0x2e, 0x7d, 0x32),
                                                "vollständig",
                                            );
                                        }
                                        Status::Unvollstaendig(missing) => {
                                            let text = format!("fehlt: {}", missing.join(", "));
                                            ui.colored_label(
                                                egui::Color32::from_rgb(0xb2, 0x6a, 0x00),
                                                &text,
                                            )
                                            .on_hover_text(format!(
                                                "Fehlende Felder: {}",
                                                missing.join(", ")
                                            ));
                                        }
                                        Status::Fehler(e) => {
                                            let text = format!("Fehler: {e}");
                                            ui.colored_label(
                                                egui::Color32::from_rgb(0xc6, 0x28, 0x28),
                                                &text,
                                            )
                                            .on_hover_text(format!("Fehlermeldung: {e}"));
                                        }
                                    });
                                });
                            }
                        });
                });

                    self.rows = rows;
                    self.neu = neu_flag;
                });
            }
            Tab::PdfVorschau => {
                egui::CentralPanel::default().show(ui, |ui| self.tab_pdf_vorschau(ui));
            }
            Tab::VorlageAnlernen => {
                egui::CentralPanel::default().show(ui, |ui| self.tab_vorlage_anlernen(ui));
            }
        }
    }
}

/// Farbe je Feld-Schlüssel für die Bereichs-Einrahmung.
/// Berechnet (zoom, pan), um einen normalisierten Bereich (0..1) in den Canvas einzupassen.
/// `margin` = relativer Rand (z. B. 0.02). Canvas = Zielrechteck; Bild natürlich `nat_w`x`nat_h`.
/// Bereich, auf den bei „Auf Schriftfeld zoomen" eingepasst wird:
/// Vereinigung der markierten Felder, sonst Standard-Schriftfeld rechts unten.
fn schriftfeld_rect(fields: &[TeachInField]) -> NormalizedRect {
    let mut iter = fields.iter().filter_map(|f| f.rect);
    if let Some(first) = iter.next() {
        let mut x0 = first.x0;
        let mut y0 = first.y0;
        let mut x1 = first.x1;
        let mut y1 = first.y1;
        for r in iter {
            x0 = x0.min(r.x0);
            y0 = y0.min(r.y0);
            x1 = x1.max(r.x1);
            y1 = y1.max(r.y1);
        }
        // kleiner Rand
        return NormalizedRect::new(
            (x0 - 0.02).max(0.0),
            (y0 - 0.02).max(0.0),
            (x1 + 0.02).min(1.0),
            (y1 + 0.02).min(1.0),
        );
    }
    // typisches Schriftfeld rechts unten
    NormalizedRect::new(0.5, 0.80, 1.0, 1.0)
}

fn fit_normalized_rect(
    canvas: egui::Rect,
    nat_w: f32,
    nat_h: f32,
    rect: NormalizedRect,
    margin: f32,
) -> (f32, egui::Vec2) {
    let nat_w = nat_w.max(1.0);
    let nat_h = nat_h.max(1.0);
    let region_w = (rect.width() * nat_w).max(1.0);
    let region_h = (rect.height() * nat_h).max(1.0);
    let region_cx = (rect.x0 + rect.x1) * 0.5 * nat_w;
    let region_cy = (rect.y0 + rect.y1) * 0.5 * nat_h;

    let cw = canvas.width().max(1.0);
    let ch = canvas.height().max(1.0);
    let margin = margin.clamp(0.0, 0.45);
    let avail_w = (cw * (1.0 - 2.0 * margin)).max(1.0);
    let avail_h = (ch * (1.0 - 2.0 * margin)).max(1.0);
    let zoom = (avail_w / region_w).min(avail_h / region_h).max(0.01);

    // Bildursprung so, dass der Bereich in der Canvas-Mitte liegt:
    // canvas.center == canvas.min + pan + region_center * zoom
    let pan = (canvas.center() - canvas.min) - egui::vec2(region_cx * zoom, region_cy * zoom);
    (zoom, pan)
}

fn region_color(key: &str) -> egui::Color32 {
    match key {
        "zeichnung" => egui::Color32::from_rgb(33, 150, 243), // Blau
        "bezeichnung" => egui::Color32::from_rgb(76, 175, 80), // Grün
        "material" => egui::Color32::from_rgb(255, 152, 0),   // Orange
        "oberflaeche" => egui::Color32::from_rgb(156, 39, 176), // Lila
        _ => egui::Color32::from_rgb(200, 200, 200),
    }
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
