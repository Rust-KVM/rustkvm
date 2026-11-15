use std::io::Write;

use rustkvm::observability::log_file::RotatingLog;

#[test]
fn rotates_at_the_size_limit_and_keeps_one_old_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.log");
    let log = RotatingLog::open(&path, 100).unwrap();

    let line = [b'a'; 40];
    for _ in 0..2 {
        (&log).write_all(&line).unwrap();
    }
    assert_eq!(std::fs::metadata(&path).unwrap().len(), 80);
    assert!(!log.rotated_path().exists());

    // The third line would pass the limit: the first two move to `.1`.
    (&log).write_all(&line).unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().len(), 40);
    assert_eq!(std::fs::metadata(log.rotated_path()).unwrap().len(), 80);

    // A second rotation replaces the old file instead of piling up more.
    for _ in 0..2 {
        (&log).write_all(&line).unwrap();
    }
    assert_eq!(std::fs::metadata(&path).unwrap().len(), 40);
    assert_eq!(std::fs::metadata(log.rotated_path()).unwrap().len(), 80);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
}

#[test]
fn appends_to_an_existing_file_and_counts_its_size() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.log");
    std::fs::write(&path, [b'x'; 90]).unwrap();

    let log = RotatingLog::open(&path, 100).unwrap();
    (&log).write_all(&[b'y'; 20]).unwrap();

    assert_eq!(std::fs::read(log.rotated_path()).unwrap(), [b'x'; 90]);
    assert_eq!(std::fs::read(&path).unwrap(), [b'y'; 20]);
}

#[test]
fn an_event_larger_than_the_limit_is_still_written() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.log");
    let log = RotatingLog::open(&path, 10).unwrap();

    (&log).write_all(&[b'z'; 50]).unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().len(), 50);
}
