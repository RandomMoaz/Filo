use crate::dedupe;
use crate::error::Result;
use crate::model::{FileEntry, Operation};
use crate::organize::OrganizeStrategy;
use crate::{scan, Filo};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

const PREVIEW_FOLDERS: usize = 6;
const PREVIEW_EXAMPLES: usize = 3;

/// Everything the analysis treats as a threshold or a name list, so a machine
/// or a project can tune it without a rebuild. See [`AdviceConfig::from_toml_file`].
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct AdviceConfig {
    /// A file at or above this size counts as "large".
    pub large_file_bytes: u64,
    /// Untouched for at least this many days counts as "stale".
    pub stale_days: i64,
    /// Fewer files than this in a folder and reorganizing is not worth it.
    pub min_files_to_organize: usize,
    /// Exact file names that are always disposable.
    pub junk_names: Vec<String>,
    /// Extensions that are always disposable.
    pub junk_extensions: Vec<String>,
    /// Extensions that are usually disposable but worth a look first.
    pub review_extensions: Vec<String>,
    /// Empty files with these names are placeholders, not clutter.
    pub keep_empty_names: Vec<String>,
    /// Installer packages, deletable once they have been run.
    pub installer_extensions: Vec<String>,
    /// Archive formats that may already have been extracted alongside.
    pub archive_extensions: Vec<String>,
    /// Directory names to skip entirely: build output, dependencies, VCS.
    pub skip_dirs: Vec<String>,
}

impl Default for AdviceConfig {
    fn default() -> Self {
        let owned = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect();
        AdviceConfig {
            large_file_bytes: 100 * 1024 * 1024,
            stale_days: 180,
            min_files_to_organize: 4,
            junk_names: owned(&["thumbs.db", ".ds_store", "desktop.ini", ".localized"]),
            junk_extensions: owned(&["crdownload", "part", "partial", "tmp", "temp"]),
            review_extensions: owned(&["bak", "old", "log", "swp"]),
            keep_empty_names: owned(&[
                ".gitkeep",
                ".keep",
                "__init__.py",
                "py.typed",
                ".nojekyll",
            ]),
            installer_extensions: owned(&["exe", "msi", "dmg", "pkg", "deb", "rpm", "appimage"]),
            archive_extensions: owned(&["zip", "rar", "7z", "tar", "gz", "tgz", "bz2", "xz"]),
            skip_dirs: owned(&scan::NOISE_DIRS),
        }
    }
}

impl AdviceConfig {
    /// Read an `[advice]` table from a TOML file. Any key left out keeps its
    /// default, so a config only has to name what it changes.
    pub fn from_toml_file(path: &Path) -> Result<Self> {
        #[derive(Deserialize, Default)]
        struct File {
            #[serde(default)]
            advice: AdviceConfig,
        }
        let text = std::fs::read_to_string(path)?;
        Ok(toml::from_str::<File>(&text)?.advice)
    }

    /// Look for `filo.toml` or `.filo.toml` beside the folder being analysed,
    /// falling back to the built-in defaults.
    pub fn discover(dir: &Path) -> Self {
        for name in ["filo.toml", ".filo.toml"] {
            let candidate = dir.join(name);
            if candidate.is_file() {
                if let Ok(config) = Self::from_toml_file(&candidate) {
                    return config;
                }
            }
        }
        Self::default()
    }

    fn skip_dirs(&self) -> Vec<&str> {
        self.skip_dirs.iter().map(|s| s.as_str()).collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Grouping {
    Extension,
    Date,
    Size,
    ExtensionThenDate,
}

impl Grouping {
    pub const ALL: [Grouping; 4] = [
        Grouping::Extension,
        Grouping::Date,
        Grouping::Size,
        Grouping::ExtensionThenDate,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Grouping::Extension => "by file type",
            Grouping::Date => "by date",
            Grouping::Size => "by size",
            Grouping::ExtensionThenDate => "by file type, then month",
        }
    }

    pub fn flag(&self) -> &'static str {
        match self {
            Grouping::Extension => "ext",
            Grouping::Date => "date",
            Grouping::Size => "size",
            Grouping::ExtensionThenDate => "ext-date",
        }
    }

    pub fn strategy(&self) -> OrganizeStrategy {
        match self {
            Grouping::Extension => OrganizeStrategy::Extension,
            Grouping::Date => OrganizeStrategy::Date,
            Grouping::Size => OrganizeStrategy::Size,
            Grouping::ExtensionThenDate => OrganizeStrategy::ExtensionThenDate,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FolderPreview {
    /// The folder that would be created, e.g. "pdf" or "pdf/2026-09".
    pub folder: String,
    pub files: usize,
    /// A few of the files that would move into it, by name.
    pub examples: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OrganizeSuggestion {
    pub grouping: Grouping,
    pub score: f64,
    pub balance: f64,
    pub coverage: f64,
    pub folders: usize,
    pub files: usize,
    pub preview: Vec<FolderPreview>,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Safety {
    Safe,
    Review,
}

impl Safety {
    pub fn label(&self) -> &'static str {
        match self {
            Safety::Safe => "safe",
            Safety::Review => "review",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum CleanupKind {
    Duplicates,
    CopyClutter,
    Junk,
    Installers,
    ExtractedArchives,
    Empty,
    EmptyFolders,
    LargeAndStale,
}

#[derive(Debug, Clone, Serialize)]
pub struct CleanupSuggestion {
    pub kind: CleanupKind,
    pub title: String,
    pub reason: String,
    pub reclaimable: u64,
    pub safety: Safety,
    pub paths: Vec<PathBuf>,
}

/// The headline recommendation for one subfolder, so a big tree can be tidied
/// folder by folder instead of all at once.
#[derive(Debug, Clone, Serialize)]
pub struct SubfolderAdvice {
    pub path: PathBuf,
    pub name: String,
    pub files: usize,
    pub bytes: u64,
    pub best: Option<Grouping>,
    pub score: f64,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Phase {
    Scanning,
    Comparing,
    Analyzing,
}

#[derive(Debug, Clone, Serialize)]
pub struct Advice {
    pub dir: PathBuf,
    pub files_here: usize,
    pub files_below: usize,
    pub bytes_below: u64,
    /// Whether build and dependency folders were left out of the scan.
    pub skipped_noise: bool,
    pub elapsed_ms: u128,
    pub organize: Vec<OrganizeSuggestion>,
    pub cleanup: Vec<CleanupSuggestion>,
    pub subfolders: Vec<SubfolderAdvice>,
}

impl Advice {
    pub fn best_organize(&self) -> Option<&OrganizeSuggestion> {
        self.organize.first().filter(|s| s.score > 0.0)
    }

    pub fn total_reclaimable(&self) -> u64 {
        self.cleanup.iter().map(|c| c.reclaimable).sum()
    }

    pub fn safe_cleanups(&self) -> impl Iterator<Item = &CleanupSuggestion> {
        self.cleanup.iter().filter(|c| c.safety == Safety::Safe)
    }

    pub fn files_flagged(&self) -> usize {
        self.cleanup.iter().map(|c| c.paths.len()).sum()
    }
}

/// Rank the built-in grouping strategies for `entries`, best first.
///
/// Each strategy is scored `balance * coverage`, both in `0..=1`:
/// `balance` is the normalized Shannon entropy of the resulting folder sizes
/// (1.0 when every folder gets the same share), and `coverage` is the share of
/// files that land in a folder holding at least two files — a folder built for
/// a single file is clutter, not organization.
pub fn rank_groupings(entries: &[FileEntry]) -> Vec<OrganizeSuggestion> {
    let mut ranked: Vec<OrganizeSuggestion> = Grouping::ALL
        .iter()
        .map(|g| score_grouping(entries, *g))
        .collect();
    // On a tie prefer the simpler scheme: fewer folders, and the plainer
    // grouping. "by type" should not lose to "by type, then month" by accident.
    ranked.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.folders.cmp(&b.folders))
            .then_with(|| rank_of(a.grouping).cmp(&rank_of(b.grouping)))
    });
    ranked
}

fn rank_of(grouping: Grouping) -> usize {
    Grouping::ALL
        .iter()
        .position(|g| *g == grouping)
        .unwrap_or(usize::MAX)
}

fn score_grouping(entries: &[FileEntry], grouping: Grouping) -> OrganizeSuggestion {
    let strategy = grouping.strategy();
    let mut counts: HashMap<String, (usize, Vec<String>)> = HashMap::new();
    for entry in entries {
        if let Some(folder) = strategy.destination_for(entry) {
            let bucket = counts.entry(folder).or_insert((0, Vec::new()));
            bucket.0 += 1;
            if bucket.1.len() < PREVIEW_EXAMPLES {
                bucket.1.push(entry.name.clone());
            }
        }
    }

    let files = entries.len();
    let folders = counts.len();

    let mut preview: Vec<FolderPreview> = counts
        .iter()
        .map(|(folder, (files, examples))| FolderPreview {
            folder: folder.clone(),
            files: *files,
            examples: examples.clone(),
        })
        .collect();
    preview.sort_by(|a, b| b.files.cmp(&a.files).then(a.folder.cmp(&b.folder)));
    preview.truncate(PREVIEW_FOLDERS);

    if files == 0 || folders < 2 {
        let reason = if files == 0 {
            "there are no files here to sort".to_string()
        } else {
            format!("every file would land in the same folder ({folders} in total)")
        };
        return OrganizeSuggestion {
            grouping,
            score: 0.0,
            balance: 0.0,
            coverage: 0.0,
            folders,
            files,
            preview,
            reason,
        };
    }

    let total = files as f64;
    let entropy: f64 = counts
        .values()
        .map(|(count, _)| {
            let p = *count as f64 / total;
            -p * p.ln()
        })
        .sum();
    let balance = entropy / (folders as f64).ln();
    let grouped: usize = counts.values().map(|(c, _)| *c).filter(|c| *c >= 2).sum();
    let coverage = grouped as f64 / total;
    let score = balance * coverage;

    let shape = if balance >= 0.85 {
        "an even split"
    } else if balance >= 0.55 {
        "a fairly even split"
    } else {
        "one dominant folder"
    };
    let reason = format!(
        "{files} files into {folders} folders, {}% of them beside similar files, {shape}",
        (coverage * 100.0).round() as u64
    );

    OrganizeSuggestion {
        grouping,
        score,
        balance,
        coverage,
        folders,
        files,
        preview,
        reason,
    }
}

fn extension_of(entry: &FileEntry) -> String {
    entry.extension.clone().unwrap_or_default()
}

/// Strip a trailing copy marker: `report (1).pdf` and `report - Copy.pdf`
/// both reduce to `report.pdf`.
fn original_name(name: &str) -> Option<String> {
    let path = Path::new(name);
    let stem = path.file_stem()?.to_string_lossy();
    let trimmed = stem.trim_end();
    let base = if let Some(open) = trimmed.rfind(" (") {
        let inside = &trimmed[open + 2..];
        if inside.ends_with(')') && inside[..inside.len() - 1].parse::<u32>().is_ok() {
            Some(&trimmed[..open])
        } else {
            None
        }
    } else {
        let lower = trimmed.to_lowercase();
        [" - copy", " copy", "-copy"]
            .iter()
            .find_map(|suffix| {
                lower
                    .strip_suffix(suffix)
                    .map(|_| &trimmed[..trimmed.len() - suffix.len()])
            })
    }?;

    let base = base.trim_end();
    if base.is_empty() {
        return None;
    }
    Some(match path.extension() {
        Some(ext) => format!("{base}.{}", ext.to_string_lossy()),
        None => base.to_string(),
    })
}

fn looks_like_a_copy(name: &str) -> bool {
    original_name(name).is_some()
}

/// Of a set of identical files, decide which one to keep: never a file whose
/// name marks it as a copy, then the shortest name, then the earliest path.
/// Sorting alone would keep `invoice (1).pdf` over `invoice.pdf`.
fn keeper_of(paths: &[PathBuf]) -> usize {
    paths
        .iter()
        .enumerate()
        .min_by_key(|(index, path)| {
            let name = path
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            (looks_like_a_copy(&name), name.len(), name, *index)
        })
        .map(|(index, _)| index)
        .unwrap_or(0)
}

fn describe_bytes(bytes: u64) -> String {
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

struct Detector<'a> {
    config: &'a AdviceConfig,
    paths: Vec<PathBuf>,
    bytes: u64,
}

impl<'a> Detector<'a> {
    fn new(config: &'a AdviceConfig) -> Self {
        Detector {
            config,
            paths: Vec::new(),
            bytes: 0,
        }
    }

    fn take(&mut self, entry: &FileEntry) {
        self.bytes += entry.size;
        self.paths.push(entry.path.clone());
    }

    fn into_suggestion(
        self,
        kind: CleanupKind,
        safety: Safety,
        title: impl Fn(usize) -> String,
        reason: impl Fn(u64) -> String,
    ) -> Option<CleanupSuggestion> {
        if self.paths.is_empty() {
            return None;
        }
        let _ = self.config;
        Some(CleanupSuggestion {
            kind,
            title: title(self.paths.len()),
            reason: reason(self.bytes),
            reclaimable: self.bytes,
            safety,
            paths: self.paths,
        })
    }
}

impl Filo {
    /// Look at `dir` and work out how it would best be tidied: which grouping
    /// suits the files that are actually there, and what is worth deleting.
    ///
    /// Generated and vendored folders (`target`, `node_modules`, `.git`, …) are
    /// skipped — they dominate the scan time and nothing in them is worth a
    /// tidy-up suggestion. Use [`Filo::advise_everything`] to include them.
    pub fn advise(&self, dir: &Path) -> Result<Advice> {
        self.advise_with(dir, &AdviceConfig::discover(dir), true, &mut |_, _| {})
    }

    /// As [`Filo::advise`], but looks inside build and dependency folders too.
    pub fn advise_everything(&self, dir: &Path) -> Result<Advice> {
        self.advise_with(dir, &AdviceConfig::discover(dir), false, &mut |_, _| {})
    }

    /// The full analysis, with an explicit config and a progress callback that
    /// receives each phase and the number of files reached so far.
    pub fn advise_with(
        &self,
        dir: &Path,
        config: &AdviceConfig,
        skip_noise: bool,
        progress: &mut dyn FnMut(Phase, usize),
    ) -> Result<Advice> {
        let started = std::time::Instant::now();
        let data_dir = self.data_dir().to_path_buf();
        let skip = config.skip_dirs();

        progress(Phase::Scanning, 0);
        let here: Vec<FileEntry> = scan::list_dir(dir)?
            .into_iter()
            .filter(|e| !e.is_dir)
            .collect();

        // One walk feeds the cleanup checks, duplicate detection and the
        // per-subfolder plans.
        let walked = if skip_noise {
            scan::walk_files_skipping(dir, &skip)
        } else {
            scan::walk_files(dir, None)
        };
        let below: Vec<FileEntry> = walked
            .into_iter()
            .filter(|e| !e.path.starts_with(&data_dir))
            .collect();
        progress(Phase::Scanning, below.len());

        progress(Phase::Comparing, below.len());
        let organize = rank_groupings(&here);
        let cleanup = self.cleanup_suggestions(dir, &below, &data_dir, config, skip_noise)?;

        progress(Phase::Analyzing, below.len());
        let subfolders = subfolder_advice(dir, &below, config);

        Ok(Advice {
            dir: dir.to_path_buf(),
            files_here: here.len(),
            files_below: below.len(),
            bytes_below: below.iter().map(|e| e.size).sum(),
            skipped_noise: skip_noise,
            elapsed_ms: started.elapsed().as_millis(),
            organize,
            cleanup,
            subfolders,
        })
    }

    fn cleanup_suggestions(
        &self,
        dir: &Path,
        below: &[FileEntry],
        data_dir: &Path,
        config: &AdviceConfig,
        skip_noise: bool,
    ) -> Result<Vec<CleanupSuggestion>> {
        let mut out = Vec::new();

        // Every path a suggestion claims is recorded here, so no file is ever
        // proposed for deletion twice. Two suggestions covering both halves of
        // a duplicate pair would, if both were applied, delete the content
        // entirely.
        let mut claimed: HashSet<PathBuf> = HashSet::new();

        let groups = dedupe::find_duplicates_in(below);
        let mut duplicate_paths = Vec::new();
        let mut duplicate_bytes = 0u64;
        for group in &groups {
            if group.paths.iter().any(|p| p.starts_with(data_dir)) {
                continue;
            }
            let keeper = keeper_of(&group.paths);
            for (index, path) in group.paths.iter().enumerate() {
                if index == keeper {
                    continue;
                }
                claimed.insert(path.clone());
                duplicate_paths.push(path.clone());
                duplicate_bytes += group.size;
            }
        }
        if !duplicate_paths.is_empty() {
            out.push(CleanupSuggestion {
                kind: CleanupKind::Duplicates,
                title: format!("{} redundant copies", duplicate_paths.len()),
                reason: format!(
                    "{} group(s) of byte-identical files; keeping the first of each frees {}",
                    groups.len(),
                    describe_bytes(duplicate_bytes)
                ),
                reclaimable: duplicate_bytes,
                safety: Safety::Safe,
                paths: duplicate_paths,
            });
        }

        let by_name: HashSet<PathBuf> = below.iter().map(|e| e.path.clone()).collect();
        let now = Utc::now();

        let mut junk_safe = Detector::new(config);
        let mut junk_review = Detector::new(config);
        let mut copies = Detector::new(config);
        let mut installers = Detector::new(config);
        let mut archives = Detector::new(config);
        let mut empty = Detector::new(config);
        let mut stale = Detector::new(config);

        for entry in below {
            if claimed.contains(&entry.path) {
                continue;
            }
            let name = entry.name.to_lowercase();
            let ext = extension_of(entry);
            let age = now.signed_duration_since(entry.modified).num_days();

            if config.junk_names.contains(&name) || name.starts_with("~$") {
                claimed.insert(entry.path.clone());
                junk_safe.take(entry);
                continue;
            }
            if config.junk_extensions.contains(&ext) {
                claimed.insert(entry.path.clone());
                junk_safe.take(entry);
                continue;
            }
            if config.review_extensions.contains(&ext) {
                claimed.insert(entry.path.clone());
                junk_review.take(entry);
                continue;
            }
            // "report (1).pdf" next to "report.pdf" is almost always leftover.
            if let Some(original) = original_name(&entry.name) {
                let sibling = entry
                    .path
                    .parent()
                    .map(|p| p.join(&original))
                    .unwrap_or_else(|| PathBuf::from(&original));
                if by_name.contains(&sibling) {
                    claimed.insert(entry.path.clone());
                    copies.take(entry);
                    continue;
                }
            }
            if entry.size == 0 {
                if !config.keep_empty_names.contains(&name) {
                    empty.take(entry);
                }
                continue;
            }
            if config.installer_extensions.contains(&ext) && age >= config.stale_days {
                installers.take(entry);
                continue;
            }
            // An archive sitting next to a folder of the same name is spent.
            if config.archive_extensions.contains(&ext) {
                let stem = Path::new(&entry.name)
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let extracted = entry.path.parent().map(|p| p.join(&stem));
                if extracted.is_some_and(|p| p.is_dir() && !scan::is_empty_dir(&p)) {
                    archives.take(entry);
                    continue;
                }
            }
            if entry.size >= config.large_file_bytes && age >= config.stale_days {
                stale.take(entry);
            }
        }

        let stale_days = config.stale_days;
        let large = describe_bytes(config.large_file_bytes);

        out.extend(junk_safe.into_suggestion(
            CleanupKind::Junk,
            Safety::Safe,
            |n| format!("{n} throwaway file(s)"),
            |_| "system clutter and half-finished downloads that nothing depends on".to_string(),
        ));
        out.extend(copies.into_suggestion(
            CleanupKind::CopyClutter,
            Safety::Review,
            |n| format!("{n} duplicated-by-name file(s)"),
            |bytes| {
                format!(
                    "each sits next to the file it was copied from — {} in total",
                    describe_bytes(bytes)
                )
            },
        ));
        out.extend(installers.into_suggestion(
            CleanupKind::Installers,
            Safety::Review,
            |n| format!("{n} old installer(s)"),
            move |bytes| {
                format!(
                    "installer packages untouched for {stale_days}+ days, almost certainly already run — {}",
                    describe_bytes(bytes)
                )
            },
        ));
        out.extend(archives.into_suggestion(
            CleanupKind::ExtractedArchives,
            Safety::Review,
            |n| format!("{n} already-extracted archive(s)"),
            |bytes| {
                format!(
                    "each has a folder of the same name beside it — {} in total",
                    describe_bytes(bytes)
                )
            },
        ));
        out.extend(junk_review.into_suggestion(
            CleanupKind::Junk,
            Safety::Review,
            |n| format!("{n} backup or log file(s)"),
            |_| "usually disposable, but check nothing still reads them".to_string(),
        ));
        out.extend(empty.into_suggestion(
            CleanupKind::Empty,
            Safety::Review,
            |n| format!("{n} empty file(s)"),
            |_| "zero bytes, and not one of the placeholder names tools rely on".to_string(),
        ));
        out.extend(stale.into_suggestion(
            CleanupKind::LargeAndStale,
            Safety::Review,
            |n| format!("{n} large file(s) untouched for months"),
            move |bytes| {
                format!(
                    "each over {large}, unmodified for {stale_days}+ days — {} in total",
                    describe_bytes(bytes)
                )
            },
        ));

        let empty_dirs: Vec<PathBuf> = if skip_noise {
            scan::walk_dirs_skipping(dir, &config.skip_dirs())
        } else {
            scan::walk_dirs_skipping(dir, &[])
        }
        .into_iter()
        .filter(|p| !p.starts_with(data_dir) && scan::is_empty_dir(p))
        .collect();
        if !empty_dirs.is_empty() {
            out.push(CleanupSuggestion {
                kind: CleanupKind::EmptyFolders,
                title: format!("{} empty folder(s)", empty_dirs.len()),
                reason: "nothing inside them at all".to_string(),
                reclaimable: 0,
                safety: Safety::Safe,
                paths: empty_dirs,
            });
        }

        out.sort_by(|a, b| {
            b.reclaimable
                .cmp(&a.reclaimable)
                .then(b.paths.len().cmp(&a.paths.len()))
        });
        Ok(out)
    }

    /// Trash everything a cleanup suggestion names, as one undoable operation.
    pub fn apply_cleanup(&self, suggestion: &CleanupSuggestion) -> Result<Operation> {
        self.delete_many(&suggestion.paths)
    }

    /// Trash only the chosen paths from a suggestion, as one undoable operation.
    pub fn apply_cleanup_subset(
        &self,
        suggestion: &CleanupSuggestion,
        chosen: &HashSet<PathBuf>,
    ) -> Result<Operation> {
        let picked: Vec<PathBuf> = suggestion
            .paths
            .iter()
            .filter(|p| chosen.contains(*p))
            .cloned()
            .collect();
        self.delete_many(&picked)
    }
}

/// Work out the headline recommendation for each immediate subfolder.
fn subfolder_advice(
    dir: &Path,
    below: &[FileEntry],
    config: &AdviceConfig,
) -> Vec<SubfolderAdvice> {
    let mut by_folder: HashMap<PathBuf, Vec<FileEntry>> = HashMap::new();
    for entry in below {
        let Some(parent) = entry.path.parent() else {
            continue;
        };
        if parent == dir {
            continue;
        }
        // Attribute the file to the top-level subfolder it lives under.
        let Ok(relative) = parent.strip_prefix(dir) else {
            continue;
        };
        let Some(top) = relative.components().next() else {
            continue;
        };
        by_folder
            .entry(dir.join(top.as_os_str()))
            .or_default()
            .push(entry.clone());
    }

    let mut out: Vec<SubfolderAdvice> = by_folder
        .into_iter()
        .filter(|(_, files)| files.len() >= config.min_files_to_organize)
        .map(|(path, files)| {
            let ranked = rank_groupings(&files);
            let best = ranked.first().filter(|s| s.score > 0.0);
            SubfolderAdvice {
                name: path
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string()),
                path,
                files: files.len(),
                bytes: files.iter().map(|e| e.size).sum(),
                best: best.map(|s| s.grouping),
                score: best.map(|s| s.score).unwrap_or(0.0),
                reason: best
                    .map(|s| s.reason.clone())
                    .unwrap_or_else(|| "already uniform — nothing to gain".to_string()),
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.files.cmp(&a.files))
    });
    out
}
