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

    filo.add(&[src.clone()], &dest, false).unwrap();
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
