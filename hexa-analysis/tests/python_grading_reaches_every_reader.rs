//! ADR-2611251938, the paths its sealed scenarios do not reach: what else read
//! Python before §1, and which spellings of a name are not a use under §2.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use hexa_analysis::analyzer::{source_violations, ArchAnalyzer};
use hexa_analysis::ports::{ArchAnalysisPort, Language};

fn write(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

async fn dead(root: &Path) -> HashSet<String> {
    ArchAnalyzer::new(hexa_analysis::default_ast())
        .find_dead_exports(root)
        .await
        .unwrap()
        .into_iter()
        .map(|d| d.export_name)
        .collect()
}

/// A Python test file of a project that is not Python is not parsed, so it
/// cannot keep another language's dead export alive.
#[tokio::test]
async fn an_ungraded_python_test_consumes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "src/domain/count.rs", "pub fn zq_only_python_names_me() {}\n");
    write(dir.path(), "tests/test_count.py", "def test_it():\n    zq_only_python_names_me()\n");
    assert!(dead(dir.path()).await.contains("zq_only_python_names_me"));
    // The same file counts once the project says it is Python.
    write(dir.path(), "pyproject.toml", "[project]\nname = \"p\"\n");
    assert!(!dead(dir.path()).await.contains("zq_only_python_names_me"));
}

const CROSSING: &str = "from ..adapters.secondary.store import save\n";

/// The single-file check (the edit hooks and `analyze --file`) follows the
/// grade, and counts a file a layer key covers before it is written.
#[tokio::test]
async fn the_single_file_check_follows_what_the_grade_reads() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "src/adapters/secondary/store.py", "def save():\n    pass\n");
    let ast = hexa_analysis::default_ast();
    let check = || source_violations(dir.path(), "src/domain/rule.py", CROSSING, &*ast);
    assert!(check().await.unwrap().is_empty(), "ungraded Python has no findings");

    write(dir.path(), ".hexa/project.json", r#"{"analyze":{"layers":{"src/domain/rule.py":"domain"}}}"#);
    assert_eq!(check().await.unwrap().len(), 1, "a covered file is checked before it exists");

    fs::remove_file(dir.path().join(".hexa/project.json")).unwrap();
    write(dir.path(), "requirements.txt", "requests\n");
    assert_eq!(check().await.unwrap().len(), 1);
}

/// Only a read of the module's name is a use.
#[test]
fn only_a_read_of_the_modules_name_is_a_use() {
    let ast = hexa_analysis::default_ast();
    let uses = |body: &str| {
        let source = format!("def zq():\n    return 1\n\n\n{body}");
        ast.names_used_at_home(&source, Language::Python).unwrap().contains("zq")
    };
    for not_a_use in [
        "_x = obj.zq\n",
        "_x = call(zq=1)\n",
        "def _f(zq):\n    return zq\n",
        "def _f(zq: int = 0):\n    return zq\n",
        "def _f(*zq):\n    return zq\n",
        "def _f():\n    zq = 2\n    return zq\n",
        "def _f():\n    a, zq = 1, 2\n    return zq\n",
        "def _f(items):\n    for zq in items:\n        print(zq)\n",
        "_x = [zq for zq in range(3)]\n",
        "_x = lambda zq: zq\n",
        "def _f():\n    with open('a') as zq:\n        return zq\n",
        "def _f():\n    def zq():\n        pass\n    return zq\n",
        "import os.zq\n",
        "from other import zq as _y\n",
        "def _f():\n    global zq\n",
        // An import inside a function binds the function's own name.
        "def _f():\n    from cfg import zq\n    return zq\n",
        "def _f():\n    import zq\n    return zq\n",
        "def _f():\n    from cfg import other as zq\n    return zq\n",
    ] {
        assert!(!uses(not_a_use), "not a use:\n{not_a_use}");
    }
    for a_use in [
        "_x = zq.attribute\n",
        "_x = call(key=zq)\n",
        "def _f(arg=zq):\n    return arg\n",
        "def _f(arg: zq) -> None:\n    pass\n",
        "def _f(zq=zq):\n    return 0\n",
        "def _f():\n    global zq\n    zq = zq()\n",
        "@zq\ndef _f():\n    pass\n",
        "class _C(zq):\n    pass\n",
        "_x = f'{zq()}'\n",
        "_x = [zq(i) for i in range(3)]\n",
        "def _f():\n    obj.zq = 1\n    return zq\n",
        // A comprehension's or lambda's own name is not the function's.
        "def _f(xs):\n    names = [zq for zq in xs]\n    return zq()\n",
        "def _f(xs):\n    names = {zq for zq in xs}\n    return zq()\n",
        "def _f(xs):\n    key = lambda zq: zq.name\n    return zq()\n",
    ] {
        assert!(uses(a_use), "a use:\n{a_use}");
    }
    assert!(ast.names_used_at_home("export function zq() {}\nzq();\n", Language::TypeScript).unwrap().is_empty());
}

/// A `.pyi` beside its module describes that module: its names are the
/// module's, so they are not dead on their own and do not keep the module's
/// dead names alive.
#[tokio::test]
async fn a_stub_beside_its_module_adds_no_dead_export_and_hides_none() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "pyproject.toml", "[project]\nname = \"p\"\n");
    write(dir.path(), "pkg/__init__.py", "");
    write(dir.path(), "pkg/zq_fast.py", "def zq_speed():\n    return 1\n\n\ndef zq_unused():\n    return 2\n");
    write(dir.path(), "pkg/zq_fast.pyi", "def zq_speed() -> int: ...\ndef zq_unused() -> int: ...\n");
    write(dir.path(), "pkg/zq_use.py", "from pkg.zq_fast import zq_speed\n\n_X = zq_speed()\n");
    let found = ArchAnalyzer::new(hexa_analysis::default_ast()).find_dead_exports(dir.path()).await.unwrap();
    let at: Vec<String> = found.iter().map(|d| format!("{}:{}", d.file, d.export_name)).collect();
    assert_eq!(at, vec!["pkg/zq_fast.py:zq_unused".to_string()]);
}

/// A stub with no `.py` (a compiled extension) is what an import of the module
/// reaches.
#[tokio::test]
async fn an_import_reaches_a_stub_that_has_no_module() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "pyproject.toml", "[project]\nname = \"p\"\n");
    write(dir.path(), "pkg/__init__.py", "");
    write(dir.path(), "pkg/_zq_core.pyi", "def zq_native() -> int: ...\ndef zq_never() -> int: ...\n");
    write(dir.path(), "pkg/zq_use.py", "from pkg._zq_core import zq_native\n");
    // `zq_use` names nothing else, so only the import keeps `zq_native` alive.
    let names = dead(dir.path()).await;
    assert!(!names.contains("zq_native"), "{names:?}");
    assert!(names.contains("zq_never"), "{names:?}");
    let found = ArchAnalyzer::new(hexa_analysis::default_ast()).detect_circular_deps(dir.path()).await.unwrap();
    assert!(found.is_empty());
    let source = "from ..adapters.secondary._zq_store import save\n";
    write(dir.path(), "src/adapters/secondary/_zq_store.pyi", "def save() -> None: ...\n");
    let ast = hexa_analysis::default_ast();
    let crossing = source_violations(dir.path(), "src/domain/rule.py", source, &*ast).await.unwrap();
    assert_eq!(crossing.len(), 1, "an import of a stub is an edge");
}
