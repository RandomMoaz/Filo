use chrono::{DateTime, TimeZone, Utc};
use filo_core::model::FileEntry;
use filo_core::recommend::{rank_groupings, AdviceConfig, CleanupKind, Grouping, Safety};
use filo_core::Filo;
use std::fs;
use std::path::PathBuf;
use tempfile::tempdir;

fn filo_in(root: &std::path::Path) -> Filo {
    Filo::with_data_dir(root.join(".filo")).unwrap()
}

fn at(year: i32, month: u32, day: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(year, month, day, 12, 0, 0).unwrap()
}

fn entry(name: &str, size: u64, modified: DateTime<Utc>) -> FileEntry {
    let path = PathBuf::from(name);
    FileEntry {
        extension: path
            .extension()
            .map(|s| s.to_string_lossy().to_lowercase()),
        path,
        name: name.to_string(),
        is_dir: false,
        size,
        modified,
    }
}

#[test]
fn mixed_extensions_from_one_day_are_grouped_by_type() {
    let day = at(2026, 5, 4);
    let mut entries = Vec::new();
    for i in 0..4 {
        entries.push(entry(&format!("doc_{i}.pdf"), 2048, day));
        entries.push(entry(&format!("pic_{i}.png"), 2048, day));
    }

    let ranked = rank_groupings(&entries);
    assert_eq!(ranked[0].grouping, Grouping::Extension);
    assert!(ranked[0].score > 0.9, "an even 4/4 split should score high");
    assert_eq!(
        ranked.iter().find(|s| s.grouping == Grouping::Date).unwrap().score,
        0.0,
        "one shared day gives a single folder, which is worthless"
    );
}

#[test]
fn one_extension_across_months_is_grouped_by_date() {
    let mut entries = Vec::new();
    for month in [1u32, 4, 7] {
        for i in 0..3 {
            entries.push(entry(&format!("note_{month}_{i}.txt"), 100, at(2026, month, 9)));
        }
    }

    let ranked = rank_groupings(&entries);
    assert_eq!(ranked[0].grouping, Grouping::Date);
    assert_eq!(
        ranked.iter().find(|s| s.grouping == Grouping::Extension).unwrap().score,
        0.0
    );
}

#[test]
fn folders_holding_a_single_file_drag_the_score_down() {
    let day = at(2026, 2, 2);
    let all_unique: Vec<FileEntry> = (0..6)
        .map(|i| entry(&format!("file_{i}.ext{i}"), 10, day))
        .collect();
    let ranked = rank_groupings(&all_unique);

    let by_type = ranked
        .iter()
        .find(|s| s.grouping == Grouping::Extension)
        .unwrap();
    assert_eq!(by_type.coverage, 0.0, "no file has company");
    assert_eq!(by_type.score, 0.0, "six folders of one file each is clutter");
}

#[test]
fn advice_spots_duplicates_junk_and_empties_but_spares_placeholders() {
    let dir = tempdir().unwrap();
    let filo = filo_in(dir.path());
    fs::write(dir.path().join("a.txt"), b"identical").unwrap();
    fs::write(dir.path().join("b.txt"), b"identical").unwrap();
    fs::write(dir.path().join("Thumbs.db"), b"junk").unwrap();
    fs::write(dir.path().join("notes.bak"), b"backup").unwrap();
    fs::write(dir.path().join("blank.txt"), b"").unwrap();
    fs::write(dir.path().join(".gitkeep"), b"").unwrap();

    let advice = filo.advise(dir.path()).unwrap();
    let titles: Vec<&str> = advice.cleanup.iter().map(|c| c.title.as_str()).collect();

    let duplicates = advice
        .cleanup
        .iter()
        .find(|c| c.title.contains("redundant"))
        .unwrap_or_else(|| panic!("no duplicate suggestion in {titles:?}"));
    assert_eq!(duplicates.paths.len(), 1, "one of the pair is redundant");
    assert_eq!(duplicates.safety, Safety::Safe);

    let junk = advice
        .cleanup
        .iter()
        .find(|c| c.title.contains("throwaway"))
        .unwrap_or_else(|| panic!("no junk suggestion in {titles:?}"));
    assert!(junk.paths.iter().any(|p| p.ends_with("Thumbs.db")));

    let backups = advice
        .cleanup
        .iter()
        .find(|c| c.title.contains("backup"))
        .unwrap();
    assert_eq!(backups.safety, Safety::Review, "a .bak may still be wanted");

    let empties = advice
        .cleanup
        .iter()
        .find(|c| c.title.contains("empty"))
        .unwrap_or_else(|| panic!("no empty-file suggestion in {titles:?}"));
    assert!(empties.paths.iter().any(|p| p.ends_with("blank.txt")));
    assert!(
        !empties.paths.iter().any(|p| p.ends_with(".gitkeep")),
        "placeholder files must not be recommended for deletion"
    );
}

#[test]
fn the_filo_data_directory_is_never_recommended_for_deletion() {
    let dir = tempdir().unwrap();
    let filo = filo_in(dir.path());
    fs::write(dir.path().join("x.txt"), b"same").unwrap();
    fs::write(dir.path().join("y.txt"), b"same").unwrap();
    filo.delete(&dir.path().join("y.txt"), false).unwrap();
    fs::write(dir.path().join("y.txt"), b"same").unwrap();

    let advice = filo.advise(dir.path()).unwrap();
    for item in &advice.cleanup {
        for path in &item.paths {
            assert!(
                !path.starts_with(filo.data_dir()),
                "{} lives in filo's own data directory",
                path.display()
            );
        }
    }
}

#[test]
fn applying_a_cleanup_is_a_single_undoable_operation() {
    let dir = tempdir().unwrap();
    let filo = filo_in(dir.path());
    fs::write(dir.path().join("Thumbs.db"), b"junk").unwrap();
    fs::write(dir.path().join("half.crdownload"), b"partial").unwrap();

    let advice = filo.advise(dir.path()).unwrap();
    let junk = advice
        .cleanup
        .iter()
        .find(|c| c.title.contains("throwaway"))
        .unwrap();
    assert_eq!(junk.paths.len(), 2);

    filo.apply_cleanup(junk).unwrap();
    assert!(!dir.path().join("Thumbs.db").exists());

    filo.undo().unwrap();
    assert!(dir.path().join("Thumbs.db").exists());
    assert!(dir.path().join("half.crdownload").exists());
}

#[test]
fn no_file_is_ever_proposed_for_deletion_twice() {
    let dir = tempdir().unwrap();
    let filo = filo_in(dir.path());
    // "invoice (1).pdf" is both a byte-identical duplicate and a name-marked
    // copy. If both detectors claimed it, applying both would wipe the pair.
    fs::write(dir.path().join("invoice.pdf"), b"same").unwrap();
    fs::write(dir.path().join("invoice (1).pdf"), b"same").unwrap();
    fs::write(dir.path().join("invoice (2).pdf"), b"different").unwrap();

    let advice = filo.advise(dir.path()).unwrap();
    let mut seen = std::collections::HashSet::new();
    for item in &advice.cleanup {
        for path in &item.paths {
            assert!(
                seen.insert(path.clone()),
                "{} appears in two suggestions",
                path.display()
            );
        }
    }
    assert!(
        !seen.contains(&dir.path().join("invoice.pdf")),
        "the original must never be the one proposed for deletion"
    );
    assert!(seen.contains(&dir.path().join("invoice (1).pdf")));
    assert!(seen.contains(&dir.path().join("invoice (2).pdf")));
}

#[test]
fn the_original_is_kept_not_the_copy() {
    let dir = tempdir().unwrap();
    let filo = filo_in(dir.path());
    fs::write(dir.path().join("report.pdf"), b"body").unwrap();
    fs::write(dir.path().join("report (1).pdf"), b"body").unwrap();

    let advice = filo.advise(dir.path()).unwrap();
    let duplicates = advice
        .cleanup
        .iter()
        .find(|c| c.kind == CleanupKind::Duplicates)
        .expect("the identical pair should be reported");
    assert_eq!(duplicates.paths, vec![dir.path().join("report (1).pdf")]);
}

#[test]
fn config_thresholds_change_what_is_flagged() {
    let dir = tempdir().unwrap();
    let filo = filo_in(dir.path());
    fs::write(dir.path().join("notes.bak"), b"backup").unwrap();

    let flagged = |config: &AdviceConfig| {
        filo.advise_with(dir.path(), config, true, &mut |_, _| {})
            .unwrap()
            .files_flagged()
    };

    assert_eq!(flagged(&AdviceConfig::default()), 1);

    let mut relaxed = AdviceConfig::default();
    relaxed.review_extensions.clear();
    assert_eq!(flagged(&relaxed), 0, "an empty list must flag nothing");
}

#[test]
fn config_is_read_from_a_toml_file() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("filo.toml");
    fs::write(
        &path,
        "[advice]\nstale_days = 7\njunk_extensions = [\"foo\"]\n",
    )
    .unwrap();

    let config = AdviceConfig::from_toml_file(&path).unwrap();
    assert_eq!(config.stale_days, 7);
    assert_eq!(config.junk_extensions, vec!["foo".to_string()]);
    assert_eq!(
        config.large_file_bytes,
        AdviceConfig::default().large_file_bytes,
        "keys left out keep their default"
    );
}

#[test]
fn an_extracted_archive_is_flagged_but_an_empty_folder_does_not_count() {
    let dir = tempdir().unwrap();
    let filo = filo_in(dir.path());
    fs::write(dir.path().join("bundle.zip"), b"zipdata").unwrap();
    fs::create_dir(dir.path().join("bundle")).unwrap();
    fs::write(dir.path().join("lonely.zip"), b"other").unwrap();
    fs::create_dir(dir.path().join("lonely")).unwrap();

    let has_archive = |advice: &filo_core::Advice| {
        advice
            .cleanup
            .iter()
            .any(|c| c.kind == CleanupKind::ExtractedArchives)
    };
    assert!(
        !has_archive(&filo.advise(dir.path()).unwrap()),
        "an empty folder is not evidence the archive was extracted"
    );

    fs::write(dir.path().join("bundle/inside.txt"), b"content").unwrap();
    let advice = filo.advise(dir.path()).unwrap();
    let archives = advice
        .cleanup
        .iter()
        .find(|c| c.kind == CleanupKind::ExtractedArchives)
        .expect("bundle.zip now has a populated folder beside it");
    assert_eq!(archives.paths, vec![dir.path().join("bundle.zip")]);
}

#[test]
fn empty_folders_are_reported_and_restored_by_undo() {
    let dir = tempdir().unwrap();
    let filo = filo_in(dir.path());
    let hollow = dir.path().join("hollow");
    fs::create_dir(&hollow).unwrap();

    let advice = filo.advise(dir.path()).unwrap();
    let folders = advice
        .cleanup
        .iter()
        .find(|c| c.kind == CleanupKind::EmptyFolders)
        .expect("the empty folder should be reported");
    assert!(folders.paths.contains(&hollow));

    filo.apply_cleanup(folders).unwrap();
    assert!(!hollow.exists());
    filo.undo().unwrap();
    assert!(hollow.is_dir(), "one undo puts the folder back");
}

#[test]
fn a_simpler_grouping_wins_a_tie() {
    let day = at(2026, 6, 1);
    let entries: Vec<FileEntry> = (0..3)
        .flat_map(|i| {
            [
                entry(&format!("a_{i}.pdf"), 100, day),
                entry(&format!("b_{i}.png"), 100, day),
            ]
        })
        .collect();

    let ranked = rank_groupings(&entries);
    assert_eq!(
        ranked[0].grouping,
        Grouping::Extension,
        "type and type+month score the same here, so the plainer one should win"
    );
}

#[test]
fn subfolders_get_their_own_recommendation() {
    let dir = tempdir().unwrap();
    let filo = filo_in(dir.path());
    let mixed = dir.path().join("mixed");
    fs::create_dir(&mixed).unwrap();
    for i in 0..3 {
        fs::write(mixed.join(format!("doc_{i}.pdf")), b"x").unwrap();
        fs::write(mixed.join(format!("pic_{i}.png")), b"y").unwrap();
    }

    let advice = filo.advise(dir.path()).unwrap();
    let sub = advice
        .subfolders
        .iter()
        .find(|s| s.name == "mixed")
        .expect("the subfolder should get its own plan");
    assert_eq!(sub.files, 6);
    assert_eq!(sub.best, Some(Grouping::Extension));
    assert!(sub.score > 0.9);
}

#[test]
fn a_project_root_is_recognised() {
    let dir = tempdir().unwrap();
    assert!(filo_core::scan::project_marker(dir.path()).is_none());

    fs::write(dir.path().join("Cargo.toml"), b"[package]").unwrap();
    assert_eq!(
        filo_core::scan::project_marker(dir.path()),
        Some("Cargo.toml"),
        "organizing a folder like this scatters the build"
    );
}

#[test]
fn reading_the_log_tail_does_not_parse_the_whole_file() {
    let dir = tempdir().unwrap();
    let filo = filo_in(dir.path());
    for i in 0..50 {
        filo.create(&dir.path().join(format!("f{i}.txt")), false)
            .unwrap();
    }

    let history = filo.history();
    assert_eq!(history.len().unwrap(), 50);
    assert_eq!(history.read_recent(10).unwrap().len(), 10);
    assert_eq!(
        history.read_recent(10).unwrap().last().unwrap().id,
        history.last().unwrap().unwrap().id,
        "the tail must end at the newest entry"
    );
    assert_eq!(history.read_recent(500).unwrap().len(), 50);
}
