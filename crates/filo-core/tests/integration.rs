use filo_core::{Filo, OrganizeStrategy, RenameSpec};
use std::fs;
use tempfile::tempdir;

fn filo_in(root: &std::path::Path) -> Filo {
    Filo::with_data_dir(root.join(".filo")).unwrap()
}

#[test]
fn create_and_undo() {
    let dir = tempdir().unwrap();
    let filo = filo_in(dir.path());
    let f = dir.path().join("hello.txt");

    filo.create(&f, false).unwrap();
    assert!(f.exists());

    filo.undo().unwrap();
    assert!(!f.exists(), "undo of create should remove the file");
}

#[test]
fn delete_goes_to_trash_and_undo_restores() {
    let dir = tempdir().unwrap();
    let filo = filo_in(dir.path());
    let f = dir.path().join("data.bin");
    fs::write(&f, b"payload").unwrap();

    filo.delete(&f, false).unwrap();
    assert!(!f.exists(), "delete should move the file out of place");

    filo.undo().unwrap();
    assert!(f.exists(), "undo should restore from trash");
    assert_eq!(fs::read(&f).unwrap(), b"payload");
}

#[test]
fn add_copy_and_move() {
    let dir = tempdir().unwrap();
    let filo = filo_in(dir.path());
    let src = dir.path().join("a.txt");
    fs::write(&src, b"x").unwrap();
    let dest = dir.path().join("dest");
    fs::create_dir(&dest).unwrap();

    filo.add(std::slice::from_ref(&src), &dest, false).unwrap();
    assert!(src.exists());
    assert!(dest.join("a.txt").exists());
}

#[test]
fn rename_in_place() {
    let dir = tempdir().unwrap();
    let filo = filo_in(dir.path());
    let f = dir.path().join("old.txt");
    fs::write(&f, b"x").unwrap();

    filo.rename(&f, "new.txt").unwrap();
    assert!(!f.exists());
    assert!(dir.path().join("new.txt").exists());

    filo.undo().unwrap();
    assert!(f.exists(), "undo should restore the old name");
}

#[test]
fn organize_by_extension_then_undo() {
    let dir = tempdir().unwrap();
    let filo = filo_in(dir.path());
    fs::write(dir.path().join("a.txt"), b"1").unwrap();
    fs::write(dir.path().join("b.txt"), b"2").unwrap();
    fs::write(dir.path().join("c.jpg"), b"3").unwrap();

    filo.organize(dir.path(), &OrganizeStrategy::Extension).unwrap();
    assert!(dir.path().join("txt/a.txt").exists());
    assert!(dir.path().join("jpg/c.jpg").exists());

    filo.undo().unwrap();
    assert!(dir.path().join("a.txt").exists(), "files should move back");
    assert!(!dir.path().join("txt").exists(), "empty folders should be cleaned up");
}

#[test]
fn find_duplicates_groups_identical_files() {
    let dir = tempdir().unwrap();
    let filo = filo_in(dir.path());
    fs::write(dir.path().join("one.txt"), b"identical").unwrap();
    fs::write(dir.path().join("two.txt"), b"identical").unwrap();
    fs::write(dir.path().join("diff.txt"), b"different").unwrap();

    let groups = filo.find_duplicates(dir.path()).unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].paths.len(), 2);
}

#[test]
fn redo_replays_an_undone_operation() {
    let dir = tempdir().unwrap();
    let filo = filo_in(dir.path());
    let f = dir.path().join("hello.txt");

    filo.create(&f, false).unwrap();
    filo.undo().unwrap();
    assert!(!f.exists());
    filo.redo().unwrap();
    assert!(f.exists(), "redo should recreate the file");
}

#[test]
fn redo_recreates_a_directory_not_a_file() {
    let dir = tempdir().unwrap();
    let filo = filo_in(dir.path());
    let d = dir.path().join("subdir");

    filo.create(&d, true).unwrap();
    filo.undo().unwrap();
    filo.redo().unwrap();
    assert!(d.is_dir(), "redo of a folder create must recreate a folder");
}

#[test]
fn multi_step_undo_and_redo() {
    let dir = tempdir().unwrap();
    let filo = filo_in(dir.path());
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    let c = dir.path().join("c.txt");

    filo.create(&a, false).unwrap();
    filo.create(&b, false).unwrap();
    filo.create(&c, false).unwrap();

    assert_eq!(filo.undo_all().unwrap().len(), 3);
    assert!(!a.exists() && !b.exists() && !c.exists());

    assert_eq!(filo.redo_many(2).unwrap().len(), 2);
    assert!(a.exists() && b.exists() && !c.exists());
}

#[test]
fn new_action_clears_the_redo_stack() {
    let dir = tempdir().unwrap();
    let filo = filo_in(dir.path());

    filo.create(&dir.path().join("a.txt"), false).unwrap();
    filo.undo().unwrap();
    assert!(filo.can_redo());

    filo.create(&dir.path().join("b.txt"), false).unwrap();
    assert!(!filo.can_redo(), "a new action clears redo");
}

#[test]
fn bulk_rename_pattern_is_collision_checked() {
    let dir = tempdir().unwrap();
    let filo = filo_in(dir.path());
    fs::write(dir.path().join("a.txt"), b"1").unwrap();
    fs::write(dir.path().join("b.txt"), b"2").unwrap();

    let plans = filo
        .plan_bulk_rename(dir.path(), &RenameSpec::Pattern("file_{n}".into()))
        .unwrap();
    assert_eq!(plans.len(), 2);

    filo.bulk_rename(dir.path(), &RenameSpec::Pattern("file_{n}".into()))
        .unwrap();
    assert!(dir.path().join("file_1.txt").exists());
    assert!(dir.path().join("file_2.txt").exists());
}

#[test]
fn permanent_delete_does_not_block_the_undo_stack() {
    let dir = tempdir().unwrap();
    let filo = filo_in(dir.path());
    let keep = dir.path().join("later.txt");
    let gone = dir.path().join("gone.txt");
    fs::write(&gone, b"x").unwrap();

    filo.create(&keep, false).unwrap();
    filo.delete(&gone, true).unwrap();
    assert!(!gone.exists());

    filo.undo().expect("the earlier create must still be undoable");
    assert!(!keep.exists());
}

#[test]
fn undo_skips_entries_whose_trash_was_emptied() {
    let dir = tempdir().unwrap();
    let filo = filo_in(dir.path());
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    fs::write(&a, b"a").unwrap();

    filo.create(&b, false).unwrap();
    filo.delete(&a, false).unwrap();
    filo.empty_trash().unwrap();

    assert!(filo.undo().is_err(), "the delete can no longer be reversed");
    filo.undo().expect("the create underneath must still be undoable");
    assert!(!b.exists());
}

#[test]
fn organize_does_not_overwrite_an_existing_destination_file() {
    let dir = tempdir().unwrap();
    let filo = filo_in(dir.path());
    fs::create_dir(dir.path().join("txt")).unwrap();
    fs::write(dir.path().join("txt/notes.txt"), b"original").unwrap();
    fs::write(dir.path().join("notes.txt"), b"loose").unwrap();

    filo.organize(dir.path(), &OrganizeStrategy::Extension).unwrap();

    let kept = fs::read(dir.path().join("txt/notes.txt")).unwrap();
    assert_eq!(kept, b"original", "the existing file must survive");
    assert!(dir.path().join("notes.txt").exists(), "the loose file stays put");
}

#[test]
fn organize_records_what_it_managed_to_move_before_failing() {
    let dir = tempdir().unwrap();
    let filo = filo_in(dir.path());
    fs::write(dir.path().join("a.md"), b"a").unwrap();
    fs::write(dir.path().join("b.txt"), b"b").unwrap();
    // A plain file where the "txt" destination folder would go, so the second
    // move fails after the first has already happened.
    fs::write(dir.path().join("txt"), b"in the way").unwrap();

    assert!(filo.organize(dir.path(), &OrganizeStrategy::Extension).is_err());
    assert!(dir.path().join("md/a.md").exists());

    filo.undo().expect("the partial move must be recorded and undoable");
    assert!(dir.path().join("a.md").exists(), "undo restores the moved file");
}

#[test]
fn chained_bulk_rename_does_not_clobber() {
    let dir = tempdir().unwrap();
    let filo = filo_in(dir.path());
    fs::write(dir.path().join("b.txt"), b"content-of-b").unwrap();
    fs::write(dir.path().join("bb.txt"), b"content-of-bb").unwrap();

    filo.bulk_rename(
        dir.path(),
        &RenameSpec::Regex {
            find: "^b".into(),
            replace: "bb".into(),
        },
    )
    .unwrap();

    assert_eq!(fs::read(dir.path().join("bb.txt")).unwrap(), b"content-of-b");
    assert_eq!(
        fs::read(dir.path().join("bbb.txt")).unwrap(),
        b"content-of-bb"
    );
}

#[test]
fn pattern_rename_keeps_the_extension() {
    let dir = tempdir().unwrap();
    let filo = filo_in(dir.path());
    fs::write(dir.path().join("report.pdf"), b"x").unwrap();

    let plans = filo
        .plan_bulk_rename(dir.path(), &RenameSpec::Pattern("pdf_scan_{n}".into()))
        .unwrap();

    assert_eq!(plans.len(), 1);
    assert_eq!(
        plans[0].to.file_name().unwrap(),
        "pdf_scan_1.pdf",
        "an extension appearing inside the new name must not eat the real one"
    );
}

#[test]
fn a_corrupt_history_line_survives_an_undo() {
    let dir = tempdir().unwrap();
    let filo = filo_in(dir.path());
    let a = dir.path().join("a.txt");
    filo.create(&a, false).unwrap();

    let log = filo.history().path().to_path_buf();
    let mut text = fs::read_to_string(&log).unwrap();
    text.push_str("{not valid json at all}\n");
    fs::write(&log, &text).unwrap();

    filo.undo().unwrap();

    let after = fs::read_to_string(&log).unwrap();
    assert!(
        after.contains("not valid json"),
        "an unreadable line must not be silently discarded by the rewrite"
    );
}

#[test]
fn organize_by_date_undo_removes_the_nested_folders() {
    let dir = tempdir().unwrap();
    let filo = filo_in(dir.path());
    fs::write(dir.path().join("a.txt"), b"a").unwrap();

    filo.organize(dir.path(), &OrganizeStrategy::Date).unwrap();
    filo.undo().unwrap();

    assert!(dir.path().join("a.txt").exists());
    let leftovers: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n != "a.txt" && n != ".filo")
        .collect();
    assert!(
        leftovers.is_empty(),
        "undo should leave no empty year folder behind, found {leftovers:?}"
    );
}

#[test]
fn delete_many_is_a_single_undo() {
    let dir = tempdir().unwrap();
    let filo = filo_in(dir.path());
    let files: Vec<_> = ["a.txt", "b.txt", "c.txt"]
        .iter()
        .map(|n| {
            let p = dir.path().join(n);
            fs::write(&p, b"x").unwrap();
            p
        })
        .collect();

    filo.delete_many(&files).unwrap();
    assert!(files.iter().all(|p| !p.exists()));

    filo.undo().unwrap();
    assert!(files.iter().all(|p| p.exists()), "one undo restores them all");
    assert!(!filo.can_undo(), "the batch was a single history entry");

    filo.redo().unwrap();
    assert!(files.iter().all(|p| !p.exists()), "one redo re-trashes them all");
}

#[test]
fn a_blocked_destination_folder_names_the_path() {
    let dir = tempdir().unwrap();
    let filo = filo_in(dir.path());
    fs::write(dir.path().join("b.txt"), b"b").unwrap();
    fs::write(dir.path().join("txt"), b"in the way").unwrap();

    let err = filo
        .organize(dir.path(), &OrganizeStrategy::Extension)
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("txt"),
        "the error should name the folder it could not create, got: {err}"
    );
}
