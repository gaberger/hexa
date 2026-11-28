//! The "Hex layers" tally counts exactly the files the grade reads
//! (ADR-2611221646 §3). `graded_files_sync` is that list; the grade's own
//! count is `ArchAnalysisResult::file_count` from `hexa analyze`'s analyzer.
//! A hidden folder the grade reads (`.github/scripts/`) and a symlinked
//! folder it follows must be in the tally too.

use std::fs;
use std::path::Path;

use hexa_analysis::analyzer::{graded_files_sync, ArchAnalyzer};
use hexa_analysis::ports::ArchAnalysisPort;

fn write(root: &Path, rel: &str, contents: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, contents).unwrap();
}

fn grade_file_count(root: &Path) -> usize {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let analyzer = ArchAnalyzer::new(hexa_analysis::default_ast());
    rt.block_on(analyzer.analyze(root)).expect("analyze").file_count
}

#[test]
fn a_hidden_folder_the_grade_reads_is_in_the_tally() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(root, "pyproject.toml", "[project]\nname = \"demo\"\n");
    write(root, "demo/domain/model.py", "class Model:\n    pass\n");
    write(root, "demo/ports/repo.py", "from demo.domain.model import Model\n");
    write(root, ".github/scripts/x.py", "print('ci')\n");

    let tally = graded_files_sync(root);
    assert!(tally.iter().any(|f| f == ".github/scripts/x.py"), "tally: {tally:?}");
    assert_eq!(tally.len(), grade_file_count(root), "tally {tally:?} must be the grade's files");
}

#[test]
fn a_symlinked_folder_the_grade_follows_is_in_the_tally() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(root, "pyproject.toml", "[project]\nname = \"demo\"\n");
    write(root, "demo/domain/model.py", "class Model:\n    pass\n");
    write(root, "shared/util.py", "def f():\n    return 1\n");
    std::os::unix::fs::symlink(root.join("shared"), root.join("demo/linked")).unwrap();

    let tally = graded_files_sync(root);
    assert_eq!(tally.len(), grade_file_count(root), "tally {tally:?} must be the grade's files");
}
