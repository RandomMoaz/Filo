use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use comfy_table::{
    presets::{NOTHING, UTF8_FULL},
    Cell, Color, Table,
};
use filo_core::{Advice, AdviceConfig, Filo, OrganizeStrategy, Phase, RenameSpec, Safety};
use filo_core::rules::RuleSet;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "filo", version, about, long_about = None)]
struct Cli {
    #[arg(long, global = true, value_name = "DIR")]
    data_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Show {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long)]
        tree: bool,
    },
    History,
    Create {
        path: PathBuf,
        #[arg(long)]
        dir: bool,
    },
    Delete {
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        #[arg(long)]
        force: bool,
    },
    Add {
        #[arg(required = true, num_args = 2..)]
        paths: Vec<PathBuf>,
        #[arg(long, name = "move")]
        move_them: bool,
    },
    Rename { path: PathBuf, new_name: String },
    BulkRename {
        #[arg(default_value = ".")]
        dir: PathBuf,
        #[arg(long, conflicts_with = "regex")]
        pattern: Option<String>,
        #[arg(long, requires = "replace")]
        regex: Option<String>,
        #[arg(long)]
        replace: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
    Organize {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long, value_enum, default_value_t = By::Ext)]
        by: By,
        #[arg(long)]
        rules: Option<PathBuf>,
        #[arg(long)]
        dry_run: bool,
    },
    Dedupe {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long)]
        delete: bool,
    },
    /// Reverse recent operations (the most recent by default).
    Undo {
        /// Undo this many operations.
        #[arg(long, conflicts_with = "all")]
        count: Option<usize>,
        /// Undo everything on the undo stack.
        #[arg(long)]
        all: bool,
    },
    /// Re-apply operations that were undone (the most recent by default).
    Redo {
        /// Redo this many operations.
        #[arg(long, conflicts_with = "all")]
        count: Option<usize>,
        /// Redo everything on the redo stack.
        #[arg(long)]
        all: bool,
    },
    /// Suggest how best to tidy a folder: which grouping fits, what to delete.
    Suggest {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Print the raw analysis as JSON.
        #[arg(long, conflicts_with = "quiet")]
        json: bool,
        /// One tab-separated line per finding, for scripts.
        #[arg(long)]
        quiet: bool,
        /// Look inside build and dependency folders too (slower).
        #[arg(long)]
        all: bool,
        /// How many file names to list per suggestion.
        #[arg(long, default_value_t = 10)]
        limit: usize,
        /// Read thresholds and name lists from a TOML file.
        #[arg(long, value_name = "FILE")]
        config: Option<PathBuf>,
        /// Carry out the recommendations without asking.
        #[arg(long, value_enum, conflicts_with = "interactive")]
        apply: Option<Apply>,
        /// Step through the recommendations one at a time.
        #[arg(long, short)]
        interactive: bool,
        /// With --apply or --interactive, show the work but change nothing.
        #[arg(long)]
        dry_run: bool,
    },
    EmptyTrash,
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
enum Apply {
    /// Only the findings marked safe.
    Safe,
    /// Only the best organize plan.
    Organize,
    /// The best organize plan and every safe finding.
    All,
}

#[derive(Copy, Clone, ValueEnum)]
enum By {
    Ext,
    Date,
    Size,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let filo = match &cli.data_dir {
        Some(dir) => Filo::with_data_dir(dir.clone()),
        None => Filo::new(),
    }
    .context("failed to initialize filo (could not set up the data directory)")?;

    match cli.command {
        Command::Show { path, tree } => show(&filo, &path, tree)?,
        Command::History => history(&filo)?,
        Command::Create { path, dir } => {
            let op = filo.create(&path, dir)?;
            println!("✓ {}", op.summary());
        }
        Command::Delete { paths, force } => {
            for p in &paths {
                let op = filo.delete(p, force)?;
                println!("✓ {}", op.summary());
            }
        }
        Command::Add { mut paths, move_them } => {
            let dest = paths.pop().expect("clap guarantees >= 2 args");
            let ops = filo.add(&paths, &dest, move_them)?;
            for op in ops {
                println!("✓ {}", op.summary());
            }
        }
        Command::Rename { path, new_name } => {
            let op = filo.rename(&path, &new_name)?;
            println!("✓ {}", op.summary());
        }
        Command::BulkRename {
            dir,
            pattern,
            regex,
            replace,
            dry_run,
        } => bulk_rename(&filo, &dir, pattern, regex, replace, dry_run)?,
        Command::Organize {
            path,
            by,
            rules,
            dry_run,
        } => organize(&filo, &path, by, rules, dry_run)?,
        Command::Dedupe { path, delete } => dedupe(&filo, &path, delete)?,
        Command::Undo { count, all } => {
            let n = if all { usize::MAX } else { count.unwrap_or(1) };
            let ops = filo.undo_many(n)?;
            if ops.is_empty() {
                println!("nothing to undo");
            }
            for op in ops {
                println!("↩ undid: {}", op.summary());
            }
        }
        Command::Redo { count, all } => {
            let n = if all { usize::MAX } else { count.unwrap_or(1) };
            let ops = filo.redo_many(n)?;
            if ops.is_empty() {
                println!("nothing to redo");
            }
            for op in ops {
                println!("↪ redid: {}", op.summary());
            }
        }
        Command::Suggest {
            path,
            json,
            quiet,
            all,
            limit,
            config,
            apply,
            interactive,
            dry_run,
        } => suggest(
            &filo,
            &path,
            SuggestOpts {
                json,
                quiet,
                all,
                limit,
                config,
                apply,
                interactive,
                dry_run,
            },
        )?,
        Command::EmptyTrash => {
            let n = filo.empty_trash()?;
            println!("✓ emptied trash ({} item(s))", n);
        }
    }
    Ok(())
}

fn show(filo: &Filo, path: &std::path::Path, tree: bool) -> Result<()> {
    let _ = filo;
    if tree {
        for entry in filo_core::scan::walk_files(path, None) {
            println!("{}", entry.path.display());
        }
        return Ok(());
    }
    let entries = filo_core::scan::list_dir(path)?;
    let mut table = Table::new();
    table.load_preset(UTF8_FULL);
    table.set_header(vec!["name", "type", "size", "modified"]);
    for e in entries {
        let kind = if e.is_dir { "dir" } else { "file" };
        table.add_row(vec![
            Cell::new(&e.name),
            Cell::new(kind),
            Cell::new(human_size(e.size)),
            Cell::new(e.modified.format("%Y-%m-%d %H:%M").to_string()),
        ]);
    }
    println!("{table}");
    Ok(())
}

fn history(filo: &Filo) -> Result<()> {
    let ops = filo.history().read_all()?;
    if ops.is_empty() {
        println!("(no operations recorded yet)");
        return Ok(());
    }
    let mut table = Table::new();
    table.load_preset(UTF8_FULL);
    table.set_header(vec!["when", "operation"]);
    for op in ops {
        table.add_row(vec![
            Cell::new(op.timestamp.format("%Y-%m-%d %H:%M:%S").to_string()),
            Cell::new(op.summary()).fg(Color::Cyan),
        ]);
    }
    println!("{table}");
    Ok(())
}

fn bulk_rename(
    filo: &Filo,
    dir: &std::path::Path,
    pattern: Option<String>,
    regex: Option<String>,
    replace: Option<String>,
    dry_run: bool,
) -> Result<()> {
    let spec = if let Some(p) = pattern {
        RenameSpec::Pattern(p)
    } else if let (Some(find), Some(rep)) = (regex, replace) {
        RenameSpec::Regex { find, replace: rep }
    } else {
        anyhow::bail!("provide either --pattern or --regex with --replace");
    };

    if dry_run {
        let plans = filo.plan_bulk_rename(dir, &spec)?;
        if plans.is_empty() {
            println!("(nothing would be renamed)");
        }
        for p in plans {
            println!(
                "would rename {} -> {}",
                p.from.display(),
                p.to.file_name().unwrap_or_default().to_string_lossy()
            );
        }
    } else {
        let ops = filo.bulk_rename(dir, &spec)?;
        println!("✓ renamed {} file(s)", ops.len());
    }
    Ok(())
}

fn organize(
    filo: &Filo,
    path: &std::path::Path,
    by: By,
    rules: Option<PathBuf>,
    dry_run: bool,
) -> Result<()> {
    let strategy = if let Some(rules_path) = rules {
        OrganizeStrategy::Rules(RuleSet::from_toml_file(&rules_path)?)
    } else {
        match by {
            By::Ext => OrganizeStrategy::Extension,
            By::Date => OrganizeStrategy::Date,
            By::Size => OrganizeStrategy::Size,
        }
    };

    if dry_run {
        let batch = filo.plan_organize(path, &strategy)?;
        if batch.is_empty() {
            println!("(nothing would be organized)");
        }
        for (from, to) in batch {
            println!("would move {} -> {}", from.display(), to.display());
        }
    } else {
        let op = filo.organize(path, &strategy)?;
        println!("✓ {}", op.summary());
    }
    Ok(())
}

fn dedupe(filo: &Filo, path: &std::path::Path, delete: bool) -> Result<()> {
    if delete {
        let (groups, ops) = filo.dedupe_delete(path)?;
        println!(
            "✓ removed {} duplicate file(s) across {} group(s)",
            ops.len(),
            groups.len()
        );
    } else {
        let groups = filo.find_duplicates(path)?;
        if groups.is_empty() {
            println!("no duplicates found");
            return Ok(());
        }
        for (i, g) in groups.iter().enumerate() {
            println!(
                "\nGroup {} — {} copies, {} each ({}…)",
                i + 1,
                g.paths.len(),
                human_size(g.size),
                &g.hash[..12]
            );
            for p in &g.paths {
                println!("   {}", p.display());
            }
        }
    }
    Ok(())
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
        format!("{} {}", bytes, UNITS[unit])
    } else {
        format!("{:.1} {}", size, UNITS[unit])
    }
}

struct SuggestOpts {
    json: bool,
    quiet: bool,
    all: bool,
    limit: usize,
    config: Option<PathBuf>,
    apply: Option<Apply>,
    interactive: bool,
    dry_run: bool,
}

fn suggest(filo: &Filo, path: &std::path::Path, opts: SuggestOpts) -> Result<()> {
    let config = match &opts.config {
        Some(file) => AdviceConfig::from_toml_file(file)
            .with_context(|| format!("could not read the config at {}", file.display()))?,
        None => AdviceConfig::discover(path),
    };

    use std::io::IsTerminal;
    let chatty = !opts.json && !opts.quiet && std::io::stderr().is_terminal();
    let mut last: Option<Phase> = None;
    let advice = filo.advise_with(path, &config, !opts.all, &mut |phase, files| {
        if !chatty || last == Some(phase) {
            return;
        }
        last = Some(phase);
        let what = match phase {
            Phase::Scanning => "scanning",
            Phase::Comparing => "comparing",
            Phase::Analyzing => "analysing",
        };
        eprint!("\r  {what} {files} file(s)...            ");
    })?;
    if chatty {
        eprint!("\r                                        \r");
    }

    if opts.json {
        println!("{}", serde_json::to_string_pretty(&advice)?);
        return Ok(());
    }
    if opts.quiet {
        return print_quiet(&advice);
    }

    print_report(&advice, opts.limit);

    if opts.interactive {
        return run_interactive(filo, &advice, opts.dry_run);
    }
    if let Some(mode) = opts.apply {
        return run_apply(filo, &advice, mode, opts.dry_run);
    }
    Ok(())
}

fn print_quiet(advice: &Advice) -> Result<()> {
    if let Some(best) = advice.best_organize() {
        println!(
            "organize\t{}\t{:.3}\t{}",
            best.grouping.flag(),
            best.score,
            best.folders
        );
    }
    for item in &advice.cleanup {
        println!(
            "cleanup\t{}\t{:?}\t{}\t{}",
            item.safety.label(),
            item.kind,
            item.paths.len(),
            item.reclaimable
        );
    }
    Ok(())
}

fn print_report(advice: &Advice, limit: usize) {
    let rule = "-".repeat(74);
    println!("\n{rule}");
    println!("  filo | {}", advice.dir.display());
    println!(
        "  {} file(s) here | {} below | {} | scanned in {:.1}s{}",
        advice.files_here,
        advice.files_below,
        human_size(advice.bytes_below),
        advice.elapsed_ms as f64 / 1000.0,
        if advice.skipped_noise {
            " | build folders skipped"
        } else {
            ""
        }
    );
    println!("{rule}");

    println!("\nORGANIZE");
    match advice.best_organize() {
        None => println!("  nothing to gain - these files do not split into useful folders"),
        Some(best) => {
            for suggestion in &advice.organize {
                let marker = if suggestion.grouping == best.grouping {
                    "*"
                } else {
                    " "
                };
                println!(
                    "\n  {marker} {:<26} {:>4.0}%   {} folder(s)",
                    suggestion.grouping.label(),
                    suggestion.score * 100.0,
                    suggestion.folders
                );
                println!("      {}", suggestion.reason);
                if suggestion.score == 0.0 {
                    continue;
                }
                for folder in &suggestion.preview {
                    println!(
                        "      {:<24} {:>4} file(s)  {}",
                        format!("{}/", folder.folder),
                        folder.files,
                        folder.examples.join(", ")
                    );
                }
                if suggestion.folders > suggestion.preview.len() {
                    println!(
                        "      ...and {} more folder(s)",
                        suggestion.folders - suggestion.preview.len()
                    );
                }
            }
            println!(
                "\n  -> filo organize {} --by {}",
                advice.dir.display(),
                best.grouping.flag()
            );
        }
    }

    let worth_showing: Vec<_> = advice.subfolders.iter().filter(|s| s.score > 0.0).collect();
    if !worth_showing.is_empty() {
        println!("\nSUBFOLDERS");
        let mut table = Table::new();
        table.load_preset(NOTHING);
        table.set_header(vec!["folder", "files", "size", "best grouping", "fit"]);
        for sub in worth_showing.iter().take(10) {
            table.add_row(vec![
                Cell::new(format!("  {}/", sub.name)),
                Cell::new(sub.files),
                Cell::new(human_size(sub.bytes)),
                Cell::new(match sub.best {
                    Some(g) => g.label(),
                    None => "-",
                }),
                Cell::new(format!("{:.0}%", sub.score * 100.0)),
            ]);
        }
        println!("{table}");
        if worth_showing.len() > 10 {
            println!("  ...and {} more", worth_showing.len() - 10);
        }
    }

    println!("\nCLEAN UP");
    if advice.cleanup.is_empty() {
        println!("  nothing stood out - no duplicates, clutter or stale bulk found");
    }
    for item in &advice.cleanup {
        let (label, color) = match item.safety {
            Safety::Safe => ("safe  ", Color::Green),
            Safety::Review => ("review", Color::Yellow),
        };
        let mut head = Table::new();
        head.load_preset(NOTHING);
        head.add_row(vec![
            Cell::new(format!("  [{label}]")).fg(color),
            Cell::new(&item.title),
            Cell::new(human_size(item.reclaimable)),
        ]);
        println!("\n{head}");
        println!("      {}", item.reason);
        for p in item.paths.iter().take(limit) {
            let shown = p.strip_prefix(&advice.dir).unwrap_or(p);
            println!("        {}", shown.display());
        }
        if item.paths.len() > limit {
            println!(
                "        ...and {} more (--limit {})",
                item.paths.len() - limit,
                item.paths.len()
            );
        }
    }

    println!("\nSUMMARY");
    println!(
        "  {} suggestion(s) | {} file(s) flagged | {} reclaimable",
        advice.cleanup.len() + usize::from(advice.best_organize().is_some()),
        advice.files_flagged(),
        human_size(advice.total_reclaimable())
    );
    let safe: usize = advice.safe_cleanups().map(|c| c.paths.len()).sum();
    if safe > 0 {
        println!("  {safe} of them are marked safe - filo suggest --apply safe");
    }
    println!();
}

fn organize_preview(filo: &Filo, advice: &Advice) -> Result<()> {
    if let Some(best) = advice.best_organize() {
        let plan = filo.plan_organize(&advice.dir, &best.grouping.strategy())?;
        for (from, to) in plan.iter().take(20) {
            println!(
                "      {} -> {}",
                from.file_name().unwrap_or_default().to_string_lossy(),
                to.strip_prefix(&advice.dir).unwrap_or(to).display()
            );
        }
        if plan.len() > 20 {
            println!("      ...and {} more move(s)", plan.len() - 20);
        }
    }
    Ok(())
}

fn run_apply(filo: &Filo, advice: &Advice, mode: Apply, dry_run: bool) -> Result<()> {
    let do_organize = matches!(mode, Apply::Organize | Apply::All);
    let do_clean = matches!(mode, Apply::Safe | Apply::All);

    if do_organize {
        if let Some(best) = advice.best_organize() {
            if dry_run {
                println!("would organize {}:", advice.dir.display());
                organize_preview(filo, advice)?;
            } else {
                let op = filo.organize(&advice.dir, &best.grouping.strategy())?;
                println!("done: {}", op.summary());
            }
        }
    }
    if do_clean {
        for item in advice.cleanup.iter().filter(|c| c.safety == Safety::Safe) {
            if dry_run {
                println!("would trash {} ({} path(s))", item.title, item.paths.len());
            } else {
                let op = filo.apply_cleanup(item)?;
                println!("done: {} - {}", op.summary(), item.title);
            }
        }
    }
    if !dry_run {
        println!("\nAll of this is undoable: filo undo --all");
    }
    Ok(())
}

fn ask(prompt: &str) -> Result<String> {
    use std::io::Write;
    print!("{prompt}");
    std::io::stdout().flush()?;
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line)? == 0 {
        return Ok("q".to_string());
    }
    Ok(line.trim().to_lowercase())
}

fn run_interactive(filo: &Filo, advice: &Advice, dry_run: bool) -> Result<()> {
    println!("\nStepping through each suggestion - y = do it, n = skip, q = stop.\n");

    if let Some(best) = advice.best_organize() {
        let mut answer = ask(&format!(
            "Organize {} {}? [y/N/p=preview/q] ",
            advice.dir.display(),
            best.grouping.label()
        ))?;
        if answer == "p" {
            organize_preview(filo, advice)?;
            answer = ask("Go ahead? [y/N/q] ")?;
        }
        if answer == "q" {
            return Ok(());
        }
        if answer == "y" {
            if dry_run {
                println!("  (dry run - nothing moved)");
            } else {
                let op = filo.organize(&advice.dir, &best.grouping.strategy())?;
                println!("  done: {}", op.summary());
            }
        }
    }

    for item in &advice.cleanup {
        let mut answer = ask(&format!(
            "Trash {} [{}], freeing {}? [y/N/p=list/q] ",
            item.title,
            item.safety.label(),
            human_size(item.reclaimable)
        ))?;
        if answer == "p" {
            for p in &item.paths {
                println!("      {}", p.strip_prefix(&advice.dir).unwrap_or(p).display());
            }
            answer = ask("Go ahead? [y/N/q] ")?;
        }
        if answer == "q" {
            break;
        }
        if answer == "y" {
            if dry_run {
                println!("  (dry run - nothing trashed)");
            } else {
                let op = filo.apply_cleanup(item)?;
                println!("  done: {}", op.summary());
            }
        }
    }
    if !dry_run {
        println!("\nAll of this is undoable: filo undo --all");
    }
    Ok(())
}
