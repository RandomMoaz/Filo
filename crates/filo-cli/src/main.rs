use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use comfy_table::{presets::UTF8_FULL, Cell, Color, Table};
use filo_core::{Filo, OrganizeStrategy, RenameSpec};
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
    Undo,
    EmptyTrash,
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
        Command::Undo => {
            let op = filo.undo()?;
            println!("↩ undid: {}", op.summary());
        }
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
