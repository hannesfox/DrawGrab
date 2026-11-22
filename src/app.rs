//! Grafische Oberfläche: Dateien per Drag & Drop sammeln, auswerten, Tabelle prüfen,
//! als CSV exportieren.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use crate::{backend, extract, pdfium_loader};
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
    Process(Vec<PathBuf>),
}

enum Event {
    Done(PathBuf, Vec<(String, String)>, Vec<String>),
    Failed(PathBuf, String),
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

struct DrawGrabApp {
    rows: Vec<Row>,
    /// Dateien, die hinzugefügt wurden, aber noch nicht ausgewertet sind
    neu: Vec<PathBuf>,
    tx: Sender<Command>,
    rx: Receiver<Event>,
    status_text: String,
    reset_table_columns: bool,
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
                        Command::Process(files) => {
                            for file in files {
                                match backend::load_pages(&pdfium, &file) {
                                    Ok(pages) => {
                                        let result = extract::extract(&pages);
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
                    }
                }
            })
            .expect("Hintergrund-Thread konnte nicht gestartet werden");

        DrawGrabApp {
            rows: Vec::new(),
            neu: Vec::new(),
            tx: tx_cmd,
            rx: rx_event,
            status_text: "Bereit – ziehe Zeichnungen (PDF) hierher.".to_string(),
            reset_table_columns: false,
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
        if self.tx.send(Command::Process(files)).is_ok() {
            self.status_text = format!("{count} Datei(en) werden ausgewertet …");
        }
    }

    /// Holt Ergebnisse aus dem Hintergrund-Thread. Gibt die Anzahl neuer Ereignisse zurück.
    fn poll_events(&mut self) -> usize {
        let mut processed = 0usize;
        while let Ok(event) = self.rx.try_recv() {
            match event {
                Event::Done(path, values, missing) => {
                    if let Some(row) = self.rows.iter_mut().find(|r| r.path == path) {
                        for (key, value) in values {
                            match key.as_str() {
                                "zeichnung" => row.nummer = value,
                                "bezeichnung" => row.bezeichnung = value,
                                "material" => row.material = value,
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

    fn export_csv(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("CSV exportieren")
            .add_filter("CSV-Datei", &["csv"])
            .set_file_name("zeichnungen.csv")
            .save_file()
        else {
            return;
        };

        let mut buffer = csv::WriterBuilder::new()
            .delimiter(b';')
            .from_writer(Vec::new());

        let _ = buffer.write_record([
            "Datei",
            "Zeichnungsnummer",
            "Bezeichnung",
            "Material",
            "Oberflächenbehandlung",
        ]);
        for row in &self.rows {
            let _ = buffer.write_record([
                row.file_name.as_str(),
                row.nummer.as_str(),
                row.bezeichnung.as_str(),
                row.material.as_str(),
                row.oberflaeche.as_str(),
            ]);
        }
        let bytes = buffer.into_inner().unwrap_or_default();

        // Byte Order Mark, damit Excel die Umlaute richtig erkennt
        let write_result = (|| -> std::io::Result<()> {
            use std::io::Write;
            let mut file = std::fs::File::create(&path)?;
            file.write_all(&[0xEF, 0xBB, 0xBF])?;
            file.write_all(&bytes)?;
            file.flush()
        })();

        match write_result {
            Ok(_) => self.status_text = format!("CSV exportiert: {}", path.display()),
            Err(e) => self.status_text = format!("CSV konnte nicht gespeichert werden: {e}"),
        }
    }
}

impl eframe::App for DrawGrabApp {
    /// Verarbeitung ohne UI – auch wenn das Fenster gerade im Hintergrund ist.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.poll_events() > 0 {
            ctx.request_repaint();
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

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

                let auswertbar = !self.neu.is_empty();
                let button =
                    egui::Button::new("▶ Alle auswerten").min_size(egui::vec2(140.0, 30.0));
                if ui.add_enabled(auswertbar, button).clicked() {
                    self.start_processing();
                }

                ui.separator();

                let button =
                    egui::Button::new("⬇ CSV exportieren").min_size(egui::vec2(150.0, 30.0));
                if ui.add_enabled(!self.rows.is_empty(), button).clicked() {
                    self.export_csv();
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
                                            "Datei: {file_name}
Pfad: {}",
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
