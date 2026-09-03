use eframe::egui::{self, Color32, RichText};
use egui_extras::{Column, TableBuilder};
use filo_core::{DuplicateGroup, FileEntry, Filo, Operation, OrganizeStrategy};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};

pub struct FiloApp {
    filo: Filo,
    cwd: PathBuf,
    entries: Vec<FileEntry>,
    selected: HashSet<PathBuf>,
    history: Vec<Operation>,
    new_name: String,
    status: String,
    scan_rx: Option<Receiver<Result<Vec<DuplicateGroup>, String>>>,
    scanning: bool,
    dups: Vec<DuplicateGroup>,
    show_dups: bool,

    // Delete confirmation dialog.
    confirm_delete: bool,

    // Cached "is there anything to redo?" (refreshed after each action).
    can_redo: bool,
}

impl FiloApp {
    pub fn new() -> Self {
        let filo = Filo::new()
            .or_else(|_| Filo::with_data_dir(PathBuf::from(".filo")))
            .expect("could not initialize filo data directory");
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let mut app = FiloApp {
            filo,
            cwd,
            entries: Vec::new(),
            selected: HashSet::new(),
            history: Vec::new(),
            new_name: String::new(),
            status: "Ready.".to_string(),
            scan_rx: None,
            scanning: false,
            dups: Vec::new(),
            show_dups: false,
            confirm_delete: false,
            can_redo: false,
        };
        app.refresh();
        app
    }

    fn refresh(&mut self) {
        self.entries = filo_core::scan::list_dir(&self.cwd).unwrap_or_default();
        self.history = self.filo.history().read_all().unwrap_or_default();
        self.can_redo = self.filo.can_redo();
        let entries = &self.entries;
        self.selected.retain(|p| entries.iter().any(|e| &e.path == p));
    }

    fn set_result<T>(&mut self, what: &str, res: filo_core::Result<T>) {
        self.status = match res {
            Ok(_) => format!("✓ {what}"),
            Err(e) => format!("✗ {what}: {e}"),
        };
        self.refresh();
    }

    fn navigate_to(&mut self, path: PathBuf) {
        self.cwd = path;
        self.selected.clear();
        self.refresh();
    }

    fn start_duplicate_scan(&mut self) {
        if self.scanning {
            return;
        }
        let dir = self.cwd.clone();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let result = filo_core::dedupe::find_duplicates(&dir).map_err(|e| e.to_string());
            let _ = tx.send(result);
        });
        self.scan_rx = Some(rx);
        self.scanning = true;
        self.status = "Scanning for duplicates…".to_string();
    }

    fn poll_duplicate_scan(&mut self, ctx: &egui::Context) {
        if !self.scanning {
            return;
        }
        ctx.request_repaint();
        if let Some(rx) = &self.scan_rx {
            if let Ok(result) = rx.try_recv() {
                self.scanning = false;
                self.scan_rx = None;
                match result {
                    Ok(groups) => {
                        let extra: usize = groups.iter().map(|g| g.paths.len() - 1).sum();
                        self.status = format!(
                            "Found {} duplicate group(s), {} redundant file(s).",
                            groups.len(),
                            extra
                        );
                        self.dups = groups;
                        self.show_dups = true;
                    }
                    Err(e) => self.status = format!("✗ dedupe: {e}"),
                }
            }
        }
    }

    fn duplicates_window(&mut self, ctx: &egui::Context) {
        if !self.show_dups {
            return;
        }
        let mut open = self.show_dups;
        let mut delete_extras = false;
        egui::Window::new("Duplicates")
            .open(&mut open)
            .default_size([520.0, 400.0])
            .show(ctx, |ui| {
                if self.dups.is_empty() {
                    ui.label("No duplicates found. 🎉");
                    return;
                }
                ui.horizontal(|ui| {
                    let extra: usize = self.dups.iter().map(|g| g.paths.len() - 1).sum();
                    ui.label(format!(
                        "{} group(s), {} redundant file(s).",
                        self.dups.len(),
                        extra
                    ));
                    if ui
                        .button("🗑 Move extras to Trash")
                        .on_hover_text("Keeps the first file in each group, trashes the rest")
                        .clicked()
                    {
                        delete_extras = true;
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (i, g) in self.dups.iter().enumerate() {
                        ui.label(
                            RichText::new(format!(
                                "Group {} — {} copies, {} each",
                                i + 1,
                                g.paths.len(),
                                human_size(g.size)
                            ))
                            .strong(),
                        );
                        for p in &g.paths {
                            ui.label(RichText::new(format!("   {}", p.display())).monospace());
                        }
                        ui.separator();
                    }
                });
            });

        if delete_extras {
            let mut trashed = 0;
            for g in &self.dups {
                for p in g.paths.iter().skip(1) {
                    if self.filo.delete(p, false).is_ok() {
                        trashed += 1;
                    }
                }
            }
            self.status = format!("✓ moved {trashed} duplicate file(s) to trash");
            self.dups.clear();
            open = false;
            self.refresh();
        }
        self.show_dups = open;
    }

    /// A modal that lists exactly which items will be deleted, before doing it.
    fn confirm_delete_dialog(&mut self, ctx: &egui::Context) {
        if !self.confirm_delete {
            return;
        }
        // Snapshot the current selection, sorted, so the list is stable & readable.
        let mut paths: Vec<PathBuf> = self.selected.iter().cloned().collect();
        paths.sort();

        if paths.is_empty() {
            self.confirm_delete = false;
            return;
        }

        let mut do_delete = false;
        let mut cancel = false;
        egui::Window::new("Confirm delete")
            .collapsible(false)
            .resizable(true)
            .default_size([460.0, 320.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(format!(
                    "Move these {} item(s) to the trash? You can undo this afterwards.",
                    paths.len()
                ));
                ui.separator();
                egui::ScrollArea::vertical()
                    .max_height(200.0)
                    .show(ui, |ui| {
                        for p in &paths {
                            let name = p
                                .file_name()
                                .map(|s| s.to_string_lossy().into_owned())
                                .unwrap_or_else(|| p.display().to_string());
                            ui.label(RichText::new(name).monospace());
                        }
                    });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                    if ui
                        .button(RichText::new("🗑 Delete").color(Color32::from_rgb(220, 80, 80)))
                        .clicked()
                    {
                        do_delete = true;
                    }
                });
            });

        if do_delete {
            let mut n = 0;
            let mut last = Ok(());
            for p in &paths {
                match self.filo.delete(p, false) {
                    Ok(_) => n += 1,
                    Err(e) => last = Err(e),
                }
            }
            self.selected.clear();
            self.confirm_delete = false;
            self.status = match last {
                Ok(()) => format!("✓ moved {n} item(s) to trash"),
                Err(e) => format!("✗ delete (moved {n} before error): {e}"),
            };
            self.refresh();
        }
        if cancel {
            self.confirm_delete = false;
        }
    }
}

impl eframe::App for FiloApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_duplicate_scan(&ctx);
        self.top_bar(ui);
        self.status_bar(ui);
        self.history_panel(ui);
        self.file_table(ui);
        self.duplicates_window(&ctx);
        self.confirm_delete_dialog(&ctx);
    }
}

impl FiloApp {
    fn top_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                if ui.button("⬆ Up").clicked() {
                    if let Some(parent) = self.cwd.parent() {
                        self.navigate_to(parent.to_path_buf());
                    }
                }
                if ui.button("🗁 Open folder…").clicked() {
                    if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                        self.navigate_to(dir);
                    }
                }
                if ui.button("⟳ Refresh").clicked() {
                    self.refresh();
                }
                ui.separator();

                ui.label("New:");
                ui.add(egui::TextEdit::singleline(&mut self.new_name).desired_width(120.0));
                if ui.button("📄 File").clicked() && !self.new_name.is_empty() {
                    let path = self.cwd.join(&self.new_name);
                    let res = self.filo.create(&path, false);
                    self.new_name.clear();
                    self.set_result("created file", res);
                }
                if ui.button("📁 Folder").clicked() && !self.new_name.is_empty() {
                    let path = self.cwd.join(&self.new_name);
                    let res = self.filo.create(&path, true);
                    self.new_name.clear();
                    self.set_result("created folder", res);
                }
                ui.separator();

                let has_sel = !self.selected.is_empty();
                if ui.button("☑ Select all").clicked() {
                    self.selected = self.entries.iter().map(|e| e.path.clone()).collect();
                }
                if ui
                    .add_enabled(has_sel, egui::Button::new("☐ Clear"))
                    .clicked()
                {
                    self.selected.clear();
                }
                if ui
                    .add_enabled(
                        has_sel,
                        egui::Button::new(format!("🗑 Delete ({})", self.selected.len())),
                    )
                    .clicked()
                {
                    self.confirm_delete = true;
                }
                ui.separator();

                if ui.button("🧹 Organize by type").clicked() {
                    let dir = self.cwd.clone();
                    let res = self.filo.organize(&dir, &OrganizeStrategy::Extension);
                    self.set_result("organized by extension", res);
                }
                if ui
                    .add_enabled(!self.scanning, egui::Button::new("🔍 Find duplicates"))
                    .clicked()
                {
                    self.start_duplicate_scan();
                }
                if self.scanning {
                    ui.spinner();
                }
                ui.separator();

                let can_undo = !self.history.is_empty();
                if ui
                    .add_enabled(can_undo, egui::Button::new("↩ Undo"))
                    .clicked()
                {
                    let res = self.filo.undo();
                    self.set_result("undo", res);
                }
                if ui
                    .add_enabled(self.can_redo, egui::Button::new("↪ Redo"))
                    .clicked()
                {
                    let res = self.filo.redo();
                    self.set_result("redo", res);
                }
            });
            ui.add_space(2.0);
            ui.label(
                RichText::new(self.cwd.display().to_string())
                    .monospace()
                    .color(Color32::GRAY),
            );
            ui.add_space(4.0);
        });
    }

    fn history_panel(&mut self, ui: &mut egui::Ui) {
        egui::Panel::right("history")
            .resizable(true)
            .default_size(300.0)
            .show(ui, |ui| {
                ui.heading("Change log");
                ui.separator();
                if self.history.is_empty() {
                    ui.label(RichText::new("No changes yet.").italics().weak());
                    return;
                }
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for op in self.history.iter().rev() {
                        ui.label(
                            RichText::new(op.timestamp.format("%H:%M:%S").to_string())
                                .small()
                                .weak(),
                        );
                        ui.label(RichText::new(op.summary()).monospace());
                        ui.separator();
                    }
                });
            });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&self.status).small());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("{} item(s)", self.entries.len()))
                            .small()
                            .weak(),
                    );
                });
            });
        });
    }

    fn file_table(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default().show(ui, |ui| {
            let mut navigate: Option<PathBuf> = None;
            let mut toggle: Option<PathBuf> = None;

            TableBuilder::new(ui)
                .striped(true)
                .column(Column::auto().at_least(24.0))
                .column(Column::remainder().at_least(200.0))
                .column(Column::auto().at_least(60.0))
                .column(Column::auto().at_least(90.0))
                .column(Column::auto().at_least(140.0))
                .header(22.0, |mut header| {
                    header.col(|ui| {
                        ui.strong("");
                    });
                    header.col(|ui| {
                        ui.strong("Name");
                    });
                    header.col(|ui| {
                        ui.strong("Type");
                    });
                    header.col(|ui| {
                        ui.strong("Size");
                    });
                    header.col(|ui| {
                        ui.strong("Modified");
                    });
                })
                .body(|mut body| {
                    for entry in &self.entries {
                        body.row(20.0, |mut row| {
                            let is_sel = self.selected.contains(&entry.path);
                            row.col(|ui| {
                                let mut checked = is_sel;
                                if ui.checkbox(&mut checked, "").changed() {
                                    toggle = Some(entry.path.clone());
                                }
                            });
                            row.col(|ui| {
                                let icon = if entry.is_dir { "📁" } else { "📄" };
                                let label = format!("{icon} {}", entry.name);
                                let resp = ui
                                    .selectable_label(is_sel, label)
                                    .on_hover_text(if entry.is_dir {
                                        "Click to select · double-click to open"
                                    } else {
                                        "Click to select"
                                    });
                                if resp.double_clicked() && entry.is_dir {
                                    navigate = Some(entry.path.clone());
                                } else if resp.clicked() {
                                    toggle = Some(entry.path.clone());
                                }
                            });
                            row.col(|ui| {
                                ui.label(if entry.is_dir { "dir" } else { "file" });
                            });
                            row.col(|ui| {
                                ui.label(human_size(entry.size));
                            });
                            row.col(|ui| {
                                ui.label(entry.modified.format("%Y-%m-%d %H:%M").to_string());
                            });
                        });
                    }
                });

            if let Some(p) = toggle {
                if !self.selected.remove(&p) {
                    self.selected.insert(p);
                }
            }
            if let Some(p) = navigate {
                self.navigate_to(p);
            }
        });
    }
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}
