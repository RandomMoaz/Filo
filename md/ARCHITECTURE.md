# Filo — File Organizer & Manager (Rust)

> Working name: **filo**. Rename freely — every place it appears is listed in
> "Renaming the project" at the bottom.

A file-management tool written in Rust with **both a CLI and an egui GUI** on
top of one shared core library. Every operation is recorded in a change log so
you can see what happened and **undo** it.

---

## 1. Design philosophy

The single most important architectural decision: **put all the real logic in a
library crate, and make the CLI and GUI thin shells over it.**

```
                 ┌─────────────┐        ┌─────────────┐
                 │   CLI       │        │   GUI       │
                 │ (filo-cli)  │        │ (filo-gui)  │
                 └──────┬──────┘        └──────┬──────┘
                        │                      │
                        └──────────┬───────────┘
                                   ▼
                          ┌─────────────────┐
                          │   filo-core     │
                          │  (the library)  │
                          │                 │
                          │  fs ops · rules │
                          │  history · undo │
                          │  hashing · scan │
                          └─────────────────┘
```

Why this matters:
- **No duplicated logic.** "Organize" or "undo" is written once and used by both.
- **Testable.** The core has no UI, so it's easy to unit-test.
- **You get the CLI working first**, then the GUI is mostly wiring buttons to
  functions that already exist and are tested.

This is done with a **Cargo workspace** — one repo, three crates.

---

## 2. Repository layout

```
filo/
├── Cargo.toml                 # workspace manifest (lists the 3 members)
├── Cargo.lock
├── README.md
├── ARCHITECTURE.md            # this file
│
├── crates/
│   ├── filo-core/             # the library — all logic lives here
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs         # public API surface, re-exports
│   │       ├── error.rs       # FiloError, Result type
│   │       ├── ops.rs         # create / delete / move / rename / show
│   │       ├── organize.rs    # rule-based sorting into folders
│   │       ├── rules.rs       # OrganizeRule: match by ext/date/size/regex
│   │       ├── dedupe.rs      # duplicate detection (hashing)
│   │       ├── rename.rs      # bulk rename patterns
│   │       ├── history.rs     # the change log (append + read)
│   │       ├── undo.rs        # reverse an operation
│   │       ├── trash.rs       # safe-delete (move to trash, not rm)
│   │       ├── scan.rs        # walk a directory into FileEntry list
│   │       └── model.rs       # FileEntry, Operation, Change types
│   │
│   ├── filo-cli/              # command-line binary
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs        # clap parser → calls filo-core
│   │       └── commands/      # one module per subcommand (optional split)
│   │
│   └── filo-gui/              # egui desktop binary
│       ├── Cargo.toml
│       └── src/
│           ├── main.rs        # eframe::run_native
│           └── app.rs         # the App struct + update() UI loop
│
└── tests/                     # integration tests against filo-core
```

Start by building `filo-core` + `filo-cli`. Add `filo-gui` once the core is
solid — the GUI then just calls the same functions.

---

## 3. Core data model (`filo-core/src/model.rs`)

These few types are the vocabulary the whole app speaks.

```rust
/// One file or directory as seen by a scan.
pub struct FileEntry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: DateTime<Utc>,
    pub extension: Option<String>,
}

/// A single thing the tool did — the unit of the change log AND of undo.
pub struct Operation {
    pub id: Uuid,
    pub kind: OperationKind,
    pub timestamp: DateTime<Utc>,
}

pub enum OperationKind {
    Create   { path: PathBuf },
    Delete   { from: PathBuf, trashed_to: PathBuf }, // remember where it went
    Move     { from: PathBuf, to: PathBuf },
    Rename   { from: PathBuf, to: PathBuf },
    Organize { batch: Vec<(PathBuf, PathBuf)> },      // many moves, one action
}
```

The key trick for undo: **an operation stores everything needed to reverse
itself.** A delete records where the file was trashed to, so undo just moves it
back. A move stores both paths. An "organize" is a batch of moves, undone as a
group.

---

## 4. The change log & undo (the heart of your idea)

You asked to track "what changes happen" — this is that feature, done properly.

**Where it lives.** A history file in a per-project or per-user data dir, e.g.
`~/.local/share/filo/history.jsonl` (use the `directories` crate for the right
path on each OS). Format is **JSONL** — one JSON `Operation` per line. This is
append-only, human-readable, and crash-safe (a half-written last line is just
skipped).

**How it works.**
1. Every mutating function in `ops.rs` / `organize.rs` / `rename.rs` returns the
   `Operation` it performed.
2. A thin wrapper appends that `Operation` to the log before returning.
3. `filo show history` prints the log (newest first).
4. `filo undo` reads the last operation and reverses it via `undo.rs`, then
   records the undo itself (or pops it — your choice; popping is simpler to
   start).

**Safe delete.** `delete` never calls `fs::remove_*` directly. It **moves the
file into a trash folder** (`~/.local/share/filo/trash/<uuid>/`) and records
`trashed_to`. This makes delete undoable and forgiving. A separate
`filo empty-trash` permanently removes them.

This one design decision — *operations that know how to reverse themselves* —
gives you history, undo, and safe delete all at once.

---

## 5. CLI command surface (`filo-cli`, using clap derive)

```
filo <COMMAND>

  show      [PATH]                 List files (table: name, size, modified, type)
            --tree                 Show as a tree
            history                Show the change log
  create    <PATH>                 Create a file
            --dir                  Create a directory instead
  delete    <PATH>...              Safe-delete (move to trash)
            --force                Permanently delete, skip trash
  add       <SRC>... <DEST>        Copy/move files into a folder  (your "add")
            --move                 Move instead of copy
  rename    <PATH> <NEW_NAME>      Rename one file            (your "change")
  bulk-rename <GLOB>               Rename many by pattern
            --pattern "img_{n}"    {n}=counter, {name}, {ext}, {date}
            --regex 's/old/new/'   Regex replace
            --dry-run              Preview without touching anything
  organize  [PATH]                 Sort files into folders
            --by ext|date|size     Built-in strategy
            --rules rules.toml      Custom rules
            --dry-run
  dedupe    [PATH]                 Find duplicate files (by hash)
            --delete               Trash all but one of each duplicate
  undo                             Reverse the last operation
  empty-trash                      Permanently clear the trash
```

`--dry-run` on the destructive/bulk commands is worth building early — it prints
what *would* happen. It builds user trust and makes the GUI "Preview" button
trivial later.

---

## 6. Feature designs

**Organize (`organize.rs` + `rules.rs`).** A rule maps a *match* to a
*destination folder*:

```toml
# rules.toml
[[rule]]
match = { extensions = ["jpg", "png", "gif"] }
into  = "Images"

[[rule]]
match = { extensions = ["pdf", "docx"] }
into  = "Documents"

[[rule]]
match = { older_than_days = 365 }
into  = "Archive/{year}"          # {year} filled from the file's date
```

Built-in strategies (`--by ext|date|size`) are just pre-made rule sets. Organize
produces one `Organize` operation holding all the moves, so **one undo reverses
the whole sort.**

**Duplicate finder (`dedupe.rs`).** Two-pass for speed: (1) group files by size —
only same-size files *can* be duplicates; (2) within each group, hash with
**SHA-256** (`sha2` crate) and compare. Report groups; with `--delete`, keep the
first and trash the rest.

**Bulk rename (`rename.rs`).** Support a template (`{n}`, `{name}`, `{ext}`,
`{date}`) and/or a regex replace. Always compute the full plan first, check for
name collisions, and honor `--dry-run`.

---

## 7. The GUI (`filo-gui`, egui/eframe 0.36)

egui is *immediate mode*: you don't build a widget tree once, you redraw the UI
every frame from your current state. The whole app is one struct and one
`update()` method.

Planned layout:
- **Left panel** — a folder tree / current path + navigation.
- **Central panel** — the file list (a `TableBuilder`): name, size, modified,
  type, with multi-select.
- **Top bar** — buttons: Create, Delete, Organize, Dedupe, Bulk-rename, Undo.
- **Right/bottom panel** — the **change-log view**, live, with an Undo button on
  the latest entry. This is the visual payoff of your "show what changes happen".
- **Modal dialogs** — for confirms and the dry-run preview ("these 40 files will
  move — Apply / Cancel").

Skeleton:

```rust
struct FiloApp {
    cwd: PathBuf,
    entries: Vec<FileEntry>,     // from filo_core::scan
    selected: HashSet<PathBuf>,
    history: Vec<Operation>,     // from filo_core::history
    status: String,
}

// NOTE: eframe 0.36 uses `fn ui(&mut self, ui, frame)` (not the older
// `update(&mut self, ctx, frame)`), and panels are the unified `egui::Panel`
// type shown *into a Ui* rather than TopBottomPanel/SidePanel on a Context.
impl eframe::App for FiloApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("bar").show(ui, |ui| { /* buttons */ });
        egui::Panel::bottom("status").show(ui, |ui| { /* status line */ });
        egui::Panel::right("log").show(ui, |ui| { /* history + undo */ });
        egui::CentralPanel::default().show(ui, |ui| { /* file table */ });
    }
}
```

Every button just calls a `filo-core` function and refreshes `entries` +
`history`. Because the logic is already tested from the CLI, the GUI is low-risk.

---

## 8. Cargo.toml files

**Workspace root — `filo/Cargo.toml`**
```toml
[workspace]
resolver = "2"
members = ["crates/filo-core", "crates/filo-cli", "crates/filo-gui"]

[workspace.package]
version = "0.1.0"
edition = "2021"
license = "MIT"

[workspace.dependencies]
anyhow     = "1"
thiserror  = "2"
serde      = { version = "1", features = ["derive"] }
serde_json = "1"
chrono     = { version = "0.4", features = ["serde"] }
uuid       = { version = "1", features = ["v4", "serde"] }
walkdir    = "2"
directories = "6"
```

**`crates/filo-core/Cargo.toml`**
```toml
[package]
name = "filo-core"
version.workspace = true
edition.workspace = true

[dependencies]
anyhow.workspace = true
thiserror.workspace = true
serde.workspace = true
serde_json.workspace = true
chrono.workspace = true
uuid.workspace = true
walkdir.workspace = true
directories.workspace = true
sha2   = "0.10"          # duplicate hashing
regex  = "1"             # bulk-rename regex
toml   = "0.8"           # rules.toml parsing
notify = "8"             # OPTIONAL: watch-mode later

[dev-dependencies]
tempfile = "3"           # tests write to throwaway dirs
```

**`crates/filo-cli/Cargo.toml`**
```toml
[package]
name = "filo-cli"
version.workspace = true
edition.workspace = true

[[bin]]
name = "filo"
path = "src/main.rs"

[dependencies]
filo-core = { path = "../filo-core" }
clap = { version = "4.6", features = ["derive"] }
anyhow.workspace = true
comfy-table = "7"        # pretty CLI tables for `show`
```

**`crates/filo-gui/Cargo.toml`**
```toml
[package]
name = "filo-gui"
version.workspace = true
edition.workspace = true

[dependencies]
filo-core = { path = "../filo-core" }
eframe = "0.36"
egui_extras = { version = "0.36", features = ["all_loaders"] }  # TableBuilder
rfd = "0.15"             # native "open folder" dialog
```

> Version notes: pinned to current releases as of Sep 2026 — eframe/egui **0.36**,
> clap **4.6**. The `0.x` crates (egui, sha2, toml, rfd, notify) can make breaking
> changes on minor bumps, so keep egui and egui_extras on the **same** minor
> version. Run `cargo update` and check `cargo build` after any bump.

---

## 9. Suggested build order (roadmap)

**Milestone 1 — Core + CLI skeleton.** Workspace compiles; `model.rs`,
`error.rs`; `scan.rs`; `filo show` lists a directory. *You can see output.*

**Milestone 2 — Basic ops + history.** `create`, `delete` (to trash), `add`,
`rename`; append each to the JSONL log; `filo show history`. *Every action is
recorded.*

**Milestone 3 — Undo.** `undo.rs` reverses the last operation; `filo undo`.
*The signature feature works.*

**Milestone 4 — Organize + dedupe + bulk-rename**, all with `--dry-run`.

**Milestone 5 — GUI.** `filo-gui` wires the tested core to egui panels; the live
change-log panel with an Undo button.

**Milestone 6 — Polish.** Watch-mode (`notify`), config file, tests, README,
`cargo build --release` binaries.

Ship each milestone runnable before starting the next.

---

## 10. Renaming the project

If you don't want "filo", change: the folder name, `members`/`name`/`path`
fields in the four `Cargo.toml`s, the `[[bin]] name = "filo"` in `filo-cli`, the
data-dir name passed to `directories`, and the title in `filo-gui`. Nothing else
depends on the name.

---

## 11. Crates used, at a glance

| Crate | Role |
|---|---|
| `clap` | CLI argument parsing (derive macros) |
| `eframe` / `egui` / `egui_extras` | the GUI, native window, tables |
| `walkdir` | recursive directory scanning |
| `sha2` | file hashing for duplicate detection |
| `regex` | bulk-rename patterns |
| `serde` / `serde_json` | (de)serialize the change log (JSONL) |
| `toml` | parse `rules.toml` |
| `chrono` | timestamps in the log |
| `uuid` | unique operation/trash IDs |
| `directories` | correct data/trash paths per OS |
| `rfd` | native folder-picker dialog in the GUI |
| `notify` | (optional) watch-mode auto-organize |
| `comfy-table` | pretty CLI tables |
| `tempfile` | throwaway dirs in tests |
| `anyhow` / `thiserror` | ergonomic error handling |
