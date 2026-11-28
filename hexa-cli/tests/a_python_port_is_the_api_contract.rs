//! The Python API contract (ADR-2611261303), for what its sealed tests do not
//! cover. The fixture is held to the hand-written `expected.openapi.json` the
//! Rust, Go and TypeScript fixtures match. The other cases come from §1, §2,
//! §4 and §5 of the decision.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use hexa_analysis::api_contract::{build, ApiBuild, FileApi};
use hexa_analysis::domain::{HexLayer, Language};
use hexa_analysis::treesitter_api::extract;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/api")
}

fn hexa(args: &[&str]) -> Output {
    let home = tempfile::tempdir().unwrap();
    Command::new(env!("CARGO_BIN_EXE_hexa")).args(args).env("HOME", home.path()).output().unwrap()
}

fn text(out: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
}

fn built(files: &[(&str, &str)]) -> ApiBuild {
    let files: Vec<FileApi> = files
        .iter()
        .map(|(file, source)| FileApi {
            file: file.to_string(),
            layer: if file.contains("ports/") { HexLayer::Ports } else { HexLayer::Domain },
            facts: extract(source, Language::from_path(file)).unwrap(),
        })
        .collect();
    build(&files)
}

const PORT: &str = "src/ports/api.py";

/// A one-operation port after `prelude`.
fn port(prelude: &str, signature: &str) -> String {
    format!("{prelude}\n\n# @hexa:api\nclass Things(Protocol):\n    # @hexa:api GET /things\n    {signature}: ...\n")
}

fn errors(source: &str) -> Vec<String> {
    built(&[(PORT, source)]).errors.iter().map(|e| e.to_string()).collect()
}

fn assert_one_error(source: &str, word: &str) {
    let found = errors(source);
    assert!(found.len() == 1 && found[0].contains(word), "expected one error saying {word:?}: {found:?}\n{source}");
}

#[test]
fn the_python_port_produces_the_expected_document() {
    let root = fixtures().join("python");
    let out = hexa(&["api", "spec", root.to_str().unwrap(), "--out", "-"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));
    let mut got: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    got["info"].as_object_mut().unwrap().remove("x-generated-by");
    let want: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(fixtures().join("expected.openapi.json")).unwrap()).unwrap();
    assert_eq!(got, want, "{}", serde_json::to_string_pretty(&got).unwrap());
}

#[test]
fn a_docstring_with_tabs_and_escapes_is_cleaned() {
    let source = "# @hexa:api\nclass Things:\n    # @hexa:api GET /things\n    def get(self) -> str:\n        '''First \\x41.\n\n\tOne.\n\t    Two.\n        '''\n";
    let b = built(&[(PORT, source)]);
    assert!(b.errors.is_empty(), "{:?}", b.errors);
    assert_eq!(b.contract.operations[0].description, "First A.\n\nOne.\n    Two.");
}

#[test]
fn a_nullable_element_or_response_is_refused() {
    assert_one_error(&port("", "def get(self) -> list[str | None]"), "element");
    assert_one_error(&port("", "def get(self) -> dict[str, Optional[int]]"), "element");
    assert_one_error(&port("", "def get(self) -> str | None"), "response");
    assert!(errors(&port("", "def get(self) -> list[str]")).is_empty());
}

#[test]
fn a_python_class_changes_nothing_for_another_language() {
    let ts = "export interface User { id: string }\n/** @hexa:api */\nexport interface Users {\n  /** @hexa:api GET /users */\n  list(): Promise<User[]>;\n}\n";
    let other = "from dataclasses import dataclass\n\n@dataclass\nclass User:\n    name: str\n";
    let b = built(&[("src/ports/users.ts", ts), ("src/domain/user.py", other)]);
    assert!(b.errors.is_empty(), "{:?}", b.errors);
    // And a TypeScript declaration is not a schema for a Python name.
    let api = port("", "def get(self) -> User");
    let b = built(&[(PORT, &api), ("src/domain/user.ts", "export interface User { id: string }\n")]);
    assert!(b.errors.iter().any(|e| e.message.contains("not declared")), "{:?}", b.errors);
}

#[test]
fn one_name_for_two_schemas_across_languages_is_an_error_not_first_wins() {
    let ts = "export interface User { id: string }\n/** @hexa:api */\nexport interface Users {\n  /** @hexa:api GET /users */\n  list(): Promise<User>;\n}\n";
    let py = port("@dataclass\nclass User:\n    name: str", "def get(self) -> User");
    let b = built(&[("src/ports/users.ts", ts), (PORT, &py)]);
    assert!(b.errors.iter().any(|e| e.message.contains("declared more than once")), "{:?}", b.errors);
}

#[test]
fn an_alias_with_two_meanings_is_an_error_not_first_wins() {
    let a = "from m import A as Thing\n\n@dataclass\nclass A:\n    a: str\n";
    let c = "from m import B as Thing\n\n@dataclass\nclass B:\n    b: str\n";
    let api = port("from domain.a import Thing", "def get(self) -> Thing");
    let b = built(&[("src/domain/a.py", a), ("src/domain/c.py", c), (PORT, &api)]);
    assert!(b.errors.iter().any(|e| e.message.contains("more than one type")), "{:?}", b.errors);
}

#[test]
fn a_type_alias_is_not_another_files_class_of_the_same_name() {
    let class = "@dataclass\nclass Names:\n    a: str\n";
    let api = port("Names = list[str]", "def get(self) -> Names");
    let b = built(&[("src/domain/names.py", class), (PORT, &api)]);
    assert!(!b.errors.is_empty(), "the alias was resolved to another file's class");
}

#[test]
fn a_schema_class_inherited_through_another_class_is_refused_at_its_declaration() {
    let models = "class Base(BaseModel):\n    a: str\n\nclass Middle(Base):\n    b: str\n\nclass Leaf(Middle):\n    c: str\n";
    let api = port("", "def get(self) -> Leaf");
    let b = built(&[("src/domain/m.py", models), (PORT, &api)]);
    assert!(b.errors.iter().any(|e| e.file == "src/domain/m.py" && e.line == 7), "{:?}", b.errors);
}

#[test]
fn a_field_declared_in_a_branch_refuses_the_class() {
    let class = "@dataclass\nclass Holder:\n    a: str\n    if DEBUG:\n        b: str";
    assert_one_error(&port(class, "def get(self) -> Holder"), "conditionally");
}

#[test]
fn a_positional_field_default_is_read_as_pydantic_reads_it() {
    let class = "class Holder(BaseModel):\n    a: str | None = Field(None)\n    b: str = Field(...)\n    c: list[str] = []";
    let b = built(&[(PORT, &port(class, "def get(self) -> Holder"))]);
    assert!(b.errors.is_empty(), "{:?}", b.errors);
    // A dataclass `field` takes no positional default.
    let class = "@dataclass\nclass Holder:\n    a: str = field(\"x\")";
    assert_one_error(&port(class, "def get(self) -> Holder"), "cannot read");
}

#[test]
fn a_comment_between_a_decorator_and_its_method_is_a_stray_tag() {
    let source = "# @hexa:api\nclass Things:\n    @abstractmethod\n    # @hexa:api GET /things\n    def get(self) -> str: ...\n";
    let b = built(&[(PORT, source)]);
    assert!(b.contract.operations.is_empty());
    assert!(b.errors.iter().any(|e| e.line == 4), "{:?}", b.errors);
}

#[test]
fn the_api_reads_no_python_test_file_and_no_stub() {
    let dir = tempfile::tempdir().unwrap();
    let tagged = port("", "def get(self) -> str");
    for (file, source) in [
        ("pyproject.toml", "[project]\nname = \"things\"\n"),
        ("src/ports/test_api.py", tagged.as_str()),
        ("src/ports/api.pyi", tagged.as_str()),
    ] {
        let path = dir.path().join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, source).unwrap();
    }
    let out = hexa(&["api", "list", dir.path().to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(2), "{}", text(&out));
}

#[test]
fn an_enum_and_a_python_class_of_one_name_are_an_error_in_either_order() {
    let rust = "pub enum Status { Active, Done }\n/// @hexa:api service=a\npub trait A {\n    /// @hexa:api GET /a\n    fn a(&self) -> Status;\n}\n";
    let py = port("@dataclass\nclass Status:\n    code: str", "def get(self) -> Status");
    for (first, second) in [(("src/ports/a.rs", rust), (PORT, py.as_str())), ((PORT, py.as_str()), ("src/ports/a.rs", rust))] {
        let b = built(&[first, second]);
        assert!(b.errors.iter().any(|e| e.message.contains("declared more than once")), "{:?}", b.errors);
    }
}

#[test]
fn a_pydantic_dataclass_config_attribute_refuses_the_class() {
    for line in ["__pydantic_config__ = ConfigDict(alias_generator=to_camel)", "__pydantic_config__: ClassVar[ConfigDict] = ConfigDict(alias_generator=to_camel)"] {
        let class = format!("@dataclass\nclass Holder:\n    {line}\n    saved_at: str");
        let b = built(&[(PORT, &port(&class, "def get(self) -> Holder"))]);
        assert!(!b.errors.is_empty(), "published under a name the server may not send: {line}");
    }
}

#[test]
fn a_very_long_chain_of_bases_is_refused_not_a_crash() {
    let mut models = String::from("class C0(BaseModel):\n    a: str\n");
    for i in 1..30_000 {
        models.push_str(&format!("\nclass C{i}(C{}):\n    a: str\n", i - 1));
    }
    let api = port("", "def get(self) -> C29999");
    let b = built(&[("src/domain/m.py", &models), (PORT, &api)]);
    assert!(!b.errors.is_empty());
}

/// A folder the walk cannot read must not read as "no tagged ports".
#[cfg(unix)]
#[test]
fn a_tree_that_cannot_be_walked_is_an_error() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let locked = dir.path().join("volume");
    std::fs::create_dir(&locked).unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    let files = hexa_analysis::analyzer::api_files_sync(dir.path()).map_err(|e| e.kind());
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(format!("{files:?}").contains("PermissionDenied"), "{files:?}");
}
