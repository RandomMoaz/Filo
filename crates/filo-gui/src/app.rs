use eframe::egui::{self, Color32, RichText};
use egui_extras::{Column, TableBuilder};
use filo_core::{Advice, DuplicateGroup, FileEntry, Filo, Operation, OrganizeStrategy, Safety};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};

const HISTORY_SHOWN: usize = 200;

pub struct FiloApp {
    filo: Filo,
    cwd: PathBuf,
    entries: Vec<FileEntry>,
    selected: HashSet<PathBuf>,
    history: Vec<Operation>,
    history_total: usize,
    new_name: String,
    status: String,
    scan_rx: Option<Receiver<Result<Vec<DuplicateGroup>, String>>>,
    scanning: bool,
    dups: Vec<DuplicateGroup>,
    show_dups: bool,

    // Delete confirmation dialog.
    confirm_delete: bool,

    // Organize confirmation dialog.
    confirm_organize: bool,

    // Cached "is there anything to redo?" (refreshed after each action).
    can_redo: bool,

    // Background "what should I do with this folder?" analysis.
    advice_rx: Option<Receiver<Result<Advice, String>>>,
    advising: bool,
    advice: Option<Advice>,
    show_advice: bool,
    advice_picked: HashSet<PathBuf>,
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
            history_total: 0,
            new_name: String::new(),
            status: "Ready.".to_string(),
            scan_rx: None,
            scanning: false,
            dups: Vec::new(),
            show_dups: false,
            confirm_delete: false,
            confirm_organize: false,
            can_redo: false,
            advice_rx: None,
            advising: false,
            advice: None,
            show_advice: false,
            advice_picked: HashSet::new(),
        };
        app.refresh();
        app
    }

    fn refresh(&mut self) {
        self.entries = filo_core::scan::list_dir(&self.cwd).unwrap_or_default();
        // Only the tail is ever displayed, and parsing the whole log on every
        // refresh is the single slowest thing the UI does.
        self.history = self
            .filo
            .history()
            .read_recent(HISTORY_SHOWN)
            .unwrap_or_default();
        self.history_total = self.filo.history().len().unwrap_or(0);
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
            let extras: Vec<PathBuf> = self
                .dups
                .iter()
                .flat_map(|g| g.paths.iter().skip(1).cloned())
                .collect();
            let n = extras.len();
            self.status = match self.filo.delete_many(&extras) {
                Ok(_) => format!("✓ moved {n} duplicate file(s) to trash"),
                Err(e) => format!("✗ dedupe delete: {e}"),
            };
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
            let n = paths.len();
            let result = self.filo.delete_many(&paths);
            self.selected.clear();
            self.confirm_delete = false;
            self.status = match result {
                Ok(_) => format!("✓ moved {n} item(s) to trash"),
                Err(e) => format!("✗ delete: {e}"),
            };
            self.refresh();
        }
        if cancel {
            self.confirm_delete = false;
        }
    }

    /// Analyse the current folder on a background thread — it walks the whole
    /// Analyse the current folder on a background thread — it walks the whole
    /// tree and hashes files, which is far too slow for the UI thread.
    fn start_advice_scan(&mut self) {
        if self.advising {
            return;
        }
        let dir = self.cwd.clone();
        let data_dir = self.filo.data_dir().to_path_buf();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let result = Filo::with_data_dir(data_dir)
                .and_then(|filo| filo.advise(&dir))
                .map_err(|e| e.to_string());
            let _ = tx.send(result);
        });
        self.advice_rx = Some(rx);
        self.advising = true;
        self.status = "Working out what to suggest…".to_string();
    }

    fn poll_advice_scan(&mut self, ctx: &egui::Context) {
        if !self.advising {
            return;
        }
        ctx.request_repaint();
        if let Some(rx) = &self.advice_rx {
            if let Ok(result) = rx.try_recv() {
                self.advising = false;
                self.advice_rx = None;
                match result {
                    Ok(advice) => {
                        self.status = match advice.best_organize() {
                            Some(best) => format!(
                                "Suggested: organize {} · {} file(s) worth reviewing.",
                                best.grouping.label(),
                                advice.files_flagged()
                            ),
                            None => format!(
                                "Nothing worth reorganizing · {} file(s) worth reviewing.",
                                advice.files_flagged()
                            ),
                        };
                        // Everything a suggestion names starts ticked; the user
                        // unticks whatever they want to keep.
                        self.advice_picked = advice
                            .cleanup
                            .iter()
                            .flat_map(|c| c.paths.iter().cloned())
                            .collect();
                        self.advice = Some(advice);
                        self.show_advice = true;
                    }
                    Err(e) => self.status = format!("✗ suggest: {e}"),
                }
            }
        }
    }

    fn advice_window(&mut self, ctx: &egui::Context) {
        if !self.show_advice {
            return;
        }
        let advice = match self.advice.clone() {
            Some(a) => a,
            None => {
                self.show_advice = false;
                return;
            }
        };

        let mut open = self.show_advice;
        let mut apply_grouping = None;
        let mut organize_subfolder = None;
        let mut trash_index = None;
        let mut tick: Vec<(PathBuf, bool)> = Vec::new();
        let mut bulk: Option<(usize, bool)> = None;

        egui::Window::new("Suggestions")
            .open(&mut open)
            .default_size([640.0, 560.0])
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(format!(
                        "{} — {} file(s) here, {} below ({}), scanned in {:.1}s",
                        advice.dir.display(),
                        advice.files_here,
                        advice.files_below,
                        human_size(advice.bytes_below),
                        advice.elapsed_ms as f64 / 1000.0
                    ))
                    .weak(),
                );
                ui.separator();

                egui::ScrollArea::vertical().show(ui, |ui| {
                    let best = advice.best_organize().map(|b| b.grouping);
                    egui::CollapsingHeader::new(RichText::new("How to organize").strong())
                        .default_open(true)
                        .show(ui, |ui| {
                            if best.is_none() {
                                ui.label("These files do not split into useful folders.");
                            }
                            for suggestion in &advice.organize {
                                ui.horizontal(|ui| {
                                    let headline = format!(
                                        "{}  ·  {:.0}% fit  ·  {} folder(s)",
                                        suggestion.grouping.label(),
                                        suggestion.score * 100.0,
                                        suggestion.folders
                                    );
                                    let text = if Some(suggestion.grouping) == best {
                                        RichText::new(format!("★ {headline}")).strong()
                                    } else {
                                        RichText::new(headline)
                                    };
                                    ui.label(text);
                                    if ui
                                        .add_enabled(
                                            suggestion.score > 0.0,
                                            egui::Button::new("Apply"),
                                        )
                                        .clicked()
                                    {
                                        apply_grouping = Some(suggestion.grouping);
                                    }
                                });
                                ui.label(RichText::new(format!("    {}", suggestion.reason)).weak());
                                if suggestion.score > 0.0 {
                                    for folder in &suggestion.preview {
                                        ui.label(
                                            RichText::new(format!(
                                                "        {}/  {} file(s): {}",
                                                folder.folder,
                                                folder.files,
                                                folder.examples.join(", ")
                                            ))
                                            .monospace()
                                            .weak(),
                                        );
                                    }
                                    if suggestion.folders > suggestion.preview.len() {
                                        ui.label(
                                            RichText::new(format!(
                                                "        …and {} more folder(s)",
                                                suggestion.folders - suggestion.preview.len()
                                            ))
                                            .weak(),
                                        );
                                    }
                                }
                                ui.add_space(4.0);
                            }
                        });

                    let subfolders: Vec<_> =
                        advice.subfolders.iter().filter(|s| s.score > 0.0).collect();
                    if !subfolders.is_empty() {
                        egui::CollapsingHeader::new(RichText::new("Subfolders").strong())
                            .default_open(false)
                            .show(ui, |ui| {
                                for sub in subfolders {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            RichText::new(format!("{}/", sub.name)).monospace(),
                                        );
                                        ui.label(
                                            RichText::new(format!(
                                                "{} file(s), {} — {} ({:.0}%)",
                                                sub.files,
                                                human_size(sub.bytes),
                                                sub.best.map(|g| g.label()).unwrap_or("—"),
                                                sub.score * 100.0
                                            ))
                                            .weak(),
                                        );
                                        if let Some(best) = sub.best {
                                            if ui.button("Organize").clicked() {
                                                organize_subfolder =
                                                    Some((sub.path.clone(), best));
                                            }
                                        }
                                    });
                                }
                            });
                    }

                    egui::CollapsingHeader::new(RichText::new("What is worth deleting").strong())
                        .default_open(true)
                        .show(ui, |ui| {
                            if advice.cleanup.is_empty() {
                                ui.label("Nothing stood out — no duplicates, clutter or stale bulk.");
                            }
                            for (i, item) in advice.cleanup.iter().enumerate() {
                                let picked = item
                                    .paths
                                    .iter()
                                    .filter(|p| self.advice_picked.contains(*p))
                                    .count();
                                let (label, color) = match item.safety {
                                    Safety::Safe => ("safe", Color32::from_rgb(90, 170, 90)),
                                    Safety::Review => ("review", Color32::from_rgb(200, 160, 60)),
                                };
                                let header = format!(
                                    "[{label}]  {}  ·  {}",
                                    item.title,
                                    human_size(item.reclaimable)
                                );
                                egui::CollapsingHeader::new(RichText::new(header).color(color))
                                    .id_salt(("cleanup", i))
                                    .default_open(item.paths.len() <= 12)
                                    .show(ui, |ui| {
                                        ui.label(RichText::new(&item.reason).weak());
                                        ui.horizontal(|ui| {
                                            if ui.small_button("All").clicked() {
                                                bulk = Some((i, true));
                                            }
                                            if ui.small_button("None").clicked() {
                                                bulk = Some((i, false));
                                            }
                                            if ui
                                                .add_enabled(
                                                    picked > 0,
                                                    egui::Button::new(format!(
                                                        "🗑 Trash {picked} selected"
                                                    )),
                                                )
                                                .clicked()
                                            {
                                                trash_index = Some(i);
                                            }
                                        });
                                        for path in &item.paths {
                                            let shown =
                                                path.strip_prefix(&advice.dir).unwrap_or(path);
                                            let mut on = self.advice_picked.contains(path);
                                            if ui
                                                .checkbox(
                                                    &mut on,
                                                    RichText::new(shown.display().to_string())
                                                        .monospace(),
                                                )
                                                .changed()
                                            {
                                                tick.push((path.clone(), on));
                                            }
                                        }
                                    });
                            }
                        });
                });
            });

        for (path, on) in tick {
            if on {
                self.advice_picked.insert(path);
            } else {
                self.advice_picked.remove(&path);
            }
        }
        if let Some((index, on)) = bulk {
            if let Some(item) = advice.cleanup.get(index) {
                for path in &item.paths {
                    if on {
                        self.advice_picked.insert(path.clone());
                    } else {
                        self.advice_picked.remove(path);
                    }
                }
            }
        }

        if let Some(grouping) = apply_grouping {
            let dir = self.cwd.clone();
            let res = self.filo.organize(&dir, &grouping.strategy());
            self.set_result(&format!("organized {}", grouping.label()), res);
            self.advice = None;
            open = false;
        }
        if let Some((dir, grouping)) = organize_subfolder {
            let res = self.filo.organize(&dir, &grouping.strategy());
            self.set_result(&format!("organized {} {}", dir.display(), grouping.label()), res);
            self.advice = None;
            open = false;
        }
        if let Some(index) = trash_index {
            let item = advice.cleanup.get(index).cloned();
            if let Some(item) = item {
                let chosen = self.advice_picked.clone();
                let n = item.paths.iter().filter(|p| chosen.contains(*p)).count();
                self.status = match self.filo.apply_cleanup_subset(&item, &chosen) {
                    Ok(_) => format!("✓ moved {n} item(s) to trash"),
                    Err(e) => format!("✗ cleanup: {e}"),
                };
                self.advice = None;
                open = false;
                self.refresh();
            }
        }
        self.show_advice = open;
    }

    /// Organizing moves every loose file into a subfolder. That is a big change
    /// to make on one click, and on a project root it scatters the files the
    /// build depends on — so show what will happen, and say so plainly.
    fn confirm_organize_dialog(&mut self, ctx: &egui::Context) {
        if !self.confirm_organize {
            return;
        }
        let dir = self.cwd.clone();
        let plan = self
            .filo
            .plan_organize(&dir, &OrganizeStrategy::Extension)
            .unwrap_or_default();
        let marker = filo_core::scan::project_marker(&dir);

        let mut go = false;
        let mut cancel = false;
        egui::Window::new("Organize this folder?")
            .collapsible(false)
            .resizable(true)
            .default_size([480.0, 300.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                if let Some(marker) = marker {
                    ui.label(
                        RichText::new(format!(
                            "⚠ This looks like a project root — it contains {marker}."
                        ))
                        .color(Color32::from_rgb(220, 120, 60))
                        .strong(),
                    );
                    ui.label(
                        RichText::new(
                            "Moving these files into subfolders will break builds and tooling \
                             that expect them here.",
                        )
                        .weak(),
                    );
                    ui.separator();
                }

                if plan.is_empty() {
                    ui.label("Nothing would move — every file is already in place.");
                } else {
                    ui.label(format!(
                        "{} file(s) would move into subfolders of {}:",
                        plan.len(),
                        dir.display()
                    ));
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .max_height(180.0)
                        .show(ui, |ui| {
                            for (from, to) in &plan {
                                let name = from
                                    .file_name()
                                    .map(|s| s.to_string_lossy().into_owned())
                                    .unwrap_or_default();
                                let dest = to
                                    .strip_prefix(&dir)
                                    .unwrap_or(to)
                                    .parent()
                                    .map(|p| p.display().to_string())
                                    .unwrap_or_default();
                                ui.label(
                                    RichText::new(format!("{name}  →  {dest}/")).monospace(),
                                );
                            }
                        });
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                    let label = if marker.is_some() {
                        RichText::new("Organize anyway").color(Color32::from_rgb(220, 120, 60))
                    } else {
                        RichText::new("Organize")
                    };
                    if ui
                        .add_enabled(!plan.is_empty(), egui::Button::new(label))
                        .clicked()
                    {
                        go = true;
                    }
                });
            });

        if go {
            let res = self.filo.organize(&dir, &OrganizeStrategy::Extension);
            self.set_result("organized by extension", res);
            self.confirm_organize = false;
        }
        if cancel {
            self.confirm_organize = false;
        }
    }
}

impl eframe::App for FiloApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_duplicate_scan(&ctx);
        self.poll_advice_scan(&ctx);
        self.top_bar(ui);
        self.status_bar(ui);
        self.history_panel(ui);
        self.file_table(ui);
        self.duplicates_window(&ctx);
        self.confirm_delete_dialog(&ctx);
        self.confirm_organize_dialog(&ctx);
        self.advice_window(&ctx);
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
                    self.confirm_organize = true;
                }
                if ui
                    .add_enabled(!self.scanning, egui::Button::new("🔍 Find duplicates"))
                    .clicked()
                {
                    self.start_duplicate_scan();
                }
                if ui
                    .add_enabled(!self.advising, egui::Button::new("💡 Suggest"))
                    .on_hover_text("Analyse this folder and recommend what to do")
                    .clicked()
                {
                    self.start_advice_scan();
                }
                if self.scanning || self.advising {
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
                if self.history_total > self.history.len() {
                    ui.label(
                        RichText::new(format!(
                            "showing the last {} of {} entries",
                            self.history.len(),
                            self.history_total
                        ))
                        .small()
                        .weak(),
                    );
                }
                // Only the rows actually on screen are built. Laying out the
                // whole log every frame is what made the window slow to appear.
                let row_height = ui.text_style_height(&egui::TextStyle::Body) * 2.0 + 12.0;
                let newest_first: Vec<&Operation> = self.history.iter().rev().collect();
                egui::ScrollArea::vertical().show_rows(
                    ui,
                    row_height,
                    newest_first.len(),
                    |ui, range| {
                        for op in &newest_first[range] {
                            ui.label(
                                RichText::new(op.timestamp.format("%H:%M:%S").to_string())
                                    .small()
                                    .weak(),
                            );
                            ui.label(RichText::new(op.summary()).monospace());
                            ui.separator();
                        }
                    },
                );
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
