#![allow(unused, clippy::all)]
mod part_1 {
    use hexa_analysis::api_contract::{build, ApiBuild, FileApi};
    use hexa_analysis::domain::{ApiOperation, ApiPortDecl, HexLayer, Language, TypeRef};
    use hexa_analysis::treesitter_api::extract;
    use std::collections::BTreeSet;

    const FILE: &str = "src/ports/bookmarks.py";

    fn file_api(file: &str, layer: HexLayer, source: &str, lang: Language) -> FileApi {
        FileApi {
            file: file.to_string(),
            layer,
            facts: extract(source, lang).expect("source is read"),
        }
    }

    fn build_py(source: &str) -> ApiBuild {
        build(&[file_api(FILE, HexLayer::Ports, source, Language::Python)])
    }

    fn only_port(source: &str) -> ApiPortDecl {
        let facts = extract(source, Language::Python).expect("source is read");
        assert_eq!(facts.ports.len(), 1, "ports: {:?}", facts.ports);
        facts.ports[0].clone()
    }

    fn op<'a>(b: &'a ApiBuild, name: &str) -> &'a ApiOperation {
        b.contract
            .operations
            .iter()
            .find(|o| o.method_name == name)
            .unwrap_or_else(|| panic!("no operation {name}; errors: {:?}", b.errors))
    }

    fn op_names(b: &ApiBuild) -> BTreeSet<String> {
        b.contract
            .operations
            .iter()
            .map(|o| o.method_name.clone())
            .collect()
    }

    /// 1-based line of the first line containing `needle`.
    fn line_of(source: &str, needle: &str) -> usize {
        source
            .lines()
            .position(|l| l.contains(needle))
            .unwrap_or_else(|| panic!("{needle} is not in the source"))
            + 1
    }

    /// S01: Tag comments above a class and its method declare a port and an operation
    #[test]
    fn s01_tag_comments_declare_port_and_operation() {
        let source = r##"from typing import Protocol


# Keeps bookmarks.
# @hexa:api service=bookmarks
class Bookmarks(Protocol):
    # List all bookmarks.
    # @hexa:api GET /bookmarks
    # Newest come first.
    def list_all(self) -> list[str]:
        """Docstring text that is not the description."""
        ...
"##;
        let port = only_port(source);
        assert_eq!(port.name, "Bookmarks");
        assert_eq!(port.tag.as_deref().map(str::trim), Some("service=bookmarks"));
        assert_eq!(port.methods.len(), 1);
        assert_eq!(
            port.methods[0].tag.as_deref().map(str::trim),
            Some("GET /bookmarks")
        );

        let b = build_py(source);
        assert!(b.errors.is_empty(), "{:?}", b.errors);
        assert_eq!(b.ports.len(), 1);
        assert_eq!(b.contract.operations.len(), 1);
        let o = op(&b, "list_all");
        assert_eq!(o.http_method.to_uppercase(), "GET");
        assert_eq!(o.path, "/bookmarks");
        assert_eq!(
            o.description.trim(),
            "List all bookmarks.\nNewest come first."
        );
    }

    /// S02: A comment run above the first decorator holds the tag
    #[test]
    fn s02_comment_above_first_decorator_holds_tag() {
        let source = r##"from abc import ABC, abstractmethod
from typing import final


# @hexa:api service=bookmarks
@final
class Bookmarks(ABC):
    # Fetch every bookmark.
    # @hexa:api GET /bookmarks
    @abstractmethod
    @traced
    def list_all(self) -> list[str]:
        ...
"##;
        let port = only_port(source);
        assert_eq!(port.name, "Bookmarks");
        assert_eq!(port.tag.as_deref().map(str::trim), Some("service=bookmarks"));

        let b = build_py(source);
        assert!(b.errors.is_empty(), "{:?}", b.errors);
        assert_eq!(b.contract.operations.len(), 1);
        let o = op(&b, "list_all");
        assert_eq!(o.http_method.to_uppercase(), "GET");
        assert_eq!(o.path, "/bookmarks");
        assert_eq!(o.description.trim(), "Fetch every bookmark.");
    }

    /// S03: The docstring is the description when the comment run has no prose
    #[test]
    fn s03_docstring_is_cleaned_description() {
        let source = r##"from typing import Protocol


# @hexa:api service=bookmarks
class Bookmarks(Protocol):
    # @hexa:api GET /first
    # @hexa:status 404 Not found
    def first(self) -> str:
        """Summary line.

        Detail one.
            Deeper.
        Detail two.

        """
        ...

    # @hexa:api GET /second
    def second(self) -> str:
        """

        Leading summary.
          Two further in.

        """
        ...
"##;
        let b = build_py(source);
        assert!(b.errors.is_empty(), "{:?}", b.errors);
        assert_eq!(
            op(&b, "first").description,
            "Summary line.\n\nDetail one.\n    Deeper.\nDetail two."
        );
        assert_eq!(
            op(&b, "second").description,
            "Leading summary.\n  Two further in."
        );
    }

    /// S04: Untagged methods and tags inside strings or docstrings expose nothing
    #[test]
    fn s04_strings_and_docstrings_are_not_tags() {
        let source = r##"from typing import Protocol


# @hexa:api service=bookmarks
class Bookmarks(Protocol):
    # @hexa:api GET /tagged
    def tagged(self) -> str:
        ...

    def untagged(self) -> str:
        ...

    def in_docstring(self) -> str:
        """
        # @hexa:api GET /docstring
        @hexa:api GET /docstring-bare
        """
        ...

    def in_default(self, q: str = "# @hexa:api GET /default") -> str:
        ...

    def in_body(self) -> str:
        text = """
        # @hexa:api GET /body
        """
        return "# @hexa:api GET /body-line"


class Other(Protocol):
    """
    # @hexa:api other
    @hexa:api other
    """

    def thing(self) -> str:
        ...
"##;
        let facts = extract(source, Language::Python).expect("source is read");
        assert_eq!(facts.ports.len(), 1, "{:?}", facts.ports);
        assert_eq!(facts.ports[0].name, "Bookmarks");
        assert!(facts.stray_tags.is_empty(), "{:?}", facts.stray_tags);

        let b = build_py(source);
        assert!(b.errors.is_empty(), "{:?}", b.errors);
        assert_eq!(op_names(&b), BTreeSet::from(["tagged".to_string()]));
        assert_eq!(b.contract.operations.len(), 1);
    }

    /// S05: A tag on a class outside the ports layer is a rule error
    #[test]
    fn s05_tag_outside_ports_is_rule_error() {
        let python = r##"from typing import Protocol


# @hexa:api bookmarks
class Bookmarks(Protocol):
    # @hexa:api GET /bookmarks
    def list(self) -> list[str]:
        ...
"##;
        let typescript = r##"

/**
 * @hexa:api bookmarks
 */
export interface Bookmarks {
  /**
   * @hexa:api GET /bookmarks
   */
  list(): Promise<string[]>;
}
"##;
        for (layer, stem) in [
            (HexLayer::Domain, "src/domain/bookmarks"),
            (HexLayer::AdaptersPrimary, "src/adapters/primary/bookmarks"),
            (
                HexLayer::AdaptersSecondary,
                "src/adapters/secondary/bookmarks",
            ),
        ] {
            let py_file = format!("{stem}.py");
            let ts_file = format!("{stem}.ts");
            let py = build(&[file_api(&py_file, layer, python, Language::Python)]);
            let ts = build(&[file_api(&ts_file, layer, typescript, Language::TypeScript)]);
            assert!(
                !ts.errors.is_empty(),
                "TypeScript gives the rule error in {layer}"
            );
            assert!(
                !py.errors.is_empty(),
                "Python gives the rule error in {layer}"
            );
            let messages = |b: &ApiBuild| -> BTreeSet<String> {
                b.errors
                    .iter()
                    .map(|e| e.message.replace(&ts_file, &py_file))
                    .collect()
            };
            assert_eq!(messages(&py), messages(&ts), "same rule error in {layer}");
            for e in &py.errors {
                assert_eq!(e.file, py_file);
                assert!(
                    (line_of(python, "# @hexa:api bookmarks")..=line_of(python, "def list"))
                        .contains(&e.line),
                    "{e}"
                );
            }
        }

        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path();
        std::fs::create_dir_all(root.join("src/domain")).unwrap();
        std::fs::write(
            root.join("pyproject.toml"),
            "[project]\nname = \"bookmarks\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::write(root.join("src/domain/bookmarks.py"), python).unwrap();
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_hexa"))
            .args(["api", "check"])
            .arg(root)
            .output()
            .expect("hexa runs");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(!out.status.success(), "check must not pass: {text}");
        assert!(text.contains("src/domain/bookmarks.py:"), "{text}");
    }

    /// S06: A tagged overload or property is an error
    #[test]
    fn s06_tagged_overload_or_property_is_error() {
        let overload = r##"from typing import Protocol, overload


# @hexa:api service=bookmarks
class Bookmarks(Protocol):
    # @hexa:api GET /healthy
    def healthy(self) -> str:
        ...

    # @hexa:api GET /offending
    @overload
    def offending(self) -> str:
        ...
"##;
        let property = r##"from typing import Protocol


# @hexa:api service=bookmarks
class Bookmarks(Protocol):
    # @hexa:api GET /healthy
    def healthy(self) -> str:
        ...

    # @hexa:api GET /offending
    @property
    def offending(self) -> str:
        ...
"##;
        for source in [overload, property] {
            let b = build_py(source);
            let range = line_of(source, "GET /offending")..=line_of(source, "def offending");
            assert!(
                b.errors
                    .iter()
                    .any(|e| e.file == FILE && range.contains(&e.line)),
                "expected an error at lines {range:?}: {:?}",
                b.errors
            );
            assert!(
                b.errors.iter().all(|e| range.contains(&e.line)),
                "only the offending method is wrong: {:?}",
                b.errors
            );
        }
    }

    /// S07: The methods exposed are exactly those with a tag comment
    #[test]
    fn s07_exposed_methods_are_exactly_the_tagged_ones() {
        const KINDS: u64 = 8;
        let mut seed: u64 = 0x5eed;
        let mut next = move || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            seed >> 33
        };
        for round in 0..80u64 {
            let count = 1 + next() % 7;
            // One method is always tagged, at a varying position.
            let forced = next() % count;
            let mut source = String::from(
            "from abc import abstractmethod\nfrom typing import Protocol, overload\n\n\n# @hexa:api service=bookmarks\nclass Bookmarks(Protocol):\n",
        );
            let mut expected = BTreeSet::new();
            for i in 0..count {
                let kind = if i == forced {
                    next() % 2
                } else {
                    next() % KINDS
                };
                let name = format!("m{i}");
                let def = format!("    def {name}(self) -> str:\n");
                let body = "        ...\n\n";
                let method = match kind {
                    0 => {
                        expected.insert(name.clone());
                        format!(
                        "    # Prose {i}.\n    # @hexa:api GET /{name}\n    # @hexa:status 404 Not found\n{def}{body}"
                    )
                    }
                    1 => {
                        expected.insert(name.clone());
                        format!("    # @hexa:api GET /{name}\n    @abstractmethod\n{def}{body}")
                    }
                    2 => format!("{def}{body}"),
                    3 => format!(
                    "{def}        \"\"\"\n        # @hexa:api GET /{name}\n        \"\"\"\n{body}"
                ),
                    4 => format!("{def}        return \"# @hexa:api GET /{name}\"\n\n"),
                    5 => format!(
                        "    def {name}(self, q: str = \"@hexa:api GET /{name}\") -> str:\n{body}"
                    ),
                    6 => format!("    @property\n{def}{body}"),
                    _ => format!("    @overload\n{def}{body}"),
                };
                source.push_str(&method);
            }
            let b = build_py(&source);
            assert!(
                b.errors.is_empty(),
                "round {round}: {:?}\n{source}",
                b.errors
            );
            assert_eq!(op_names(&b), expected, "round {round}:\n{source}");
            assert_eq!(
                b.contract.operations.len(),
                expected.len(),
                "round {round}:\n{source}"
            );
            for o in &b.contract.operations {
                assert!(
                    !o.description.contains("@hexa:"),
                    "round {round}: {:?}",
                    o.description
                );
            }
        }
    }

    /// S08: The receiver and bare separators are not parameters
    #[test]
    fn s08_receiver_and_separators_are_not_parameters() {
        let source = r##"from typing import Protocol


# @hexa:api service=items
class Items(Protocol):
    # @hexa:api GET /items/{id}
    def fetch(this, id: str) -> str:
        ...

    # @hexa:api GET /search
    @classmethod
    def search(cls, term: str) -> str:
        ...

    # @hexa:api GET /count
    @staticmethod
    def count(owner: str) -> int:
        ...

    # @hexa:api GET /find/{id}
    def find(self, id: str, /, tag: str, *, limit: int) -> str:
        ...
"##;
        let b = build_py(source);
        assert!(b.errors.is_empty(), "{:?}", b.errors);
        assert_eq!(b.contract.operations.len(), 4);
        let cases: [(&str, &[&str]); 4] = [
            ("fetch", &["id"]),
            ("search", &["term"]),
            ("count", &["owner"]),
            ("find", &["id", "tag", "limit"]),
        ];
        let port = only_port(source);
        for (name, wanted) in cases {
            let method = port
                .methods
                .iter()
                .find(|m| m.name == name)
                .expect("method read");
            let read: Vec<&str> = method.params.iter().map(|p| p.0.as_str()).collect();
            assert_eq!(read, wanted, "{name}");
            let o = op(&b, name);
            let args: Vec<&str> = o.args.iter().map(|a| a.name.as_str()).collect();
            assert_eq!(args, wanted, "{name}");
            assert_eq!(o.params.len(), wanted.len(), "{name}: {:?}", o.params);
            assert!(o.body.is_none(), "{name}");
        }
    }

    /// S09: Sync, async and awaitable return forms give the same response
    #[test]
    fn s09_return_forms_give_the_same_response() {
        let source = r##"from dataclasses import dataclass
from typing import Any, Awaitable, Coroutine, Protocol


@dataclass
class Bookmark:
    url: str
    title: str


# @hexa:api service=bookmarks
class Bookmarks(Protocol):
    # @hexa:api POST /a/{id}
    def plain(self, id: str, bookmark: Bookmark) -> Bookmark:
        ...

    # @hexa:api POST /b/{id}
    async def asynchronous(self, id: str, bookmark: Bookmark) -> Bookmark:
        ...

    # @hexa:api POST /c/{id}
    def awaitable(self, id: str, bookmark: Bookmark) -> Awaitable[Bookmark]:
        ...

    # @hexa:api POST /d/{id}
    def coroutine(self, id: str, bookmark: Bookmark) -> Coroutine[Any, Any, Bookmark]:
        ...
"##;
        let b = build_py(source);
        assert!(b.errors.is_empty(), "{:?}", b.errors);
        assert_eq!(b.contract.operations.len(), 4);
        let first = op(&b, "plain");
        assert_eq!(first.response, Some(TypeRef::Named("Bookmark".to_string())));
        assert!(first.body.is_some());
        for name in ["asynchronous", "awaitable", "coroutine"] {
            let o = op(&b, name);
            assert_eq!(o.params, first.params, "{name}");
            assert_eq!(o.body, first.body, "{name}");
            assert_eq!(o.success, first.success, "{name}");
            assert_eq!(o.response, first.response, "{name}");
        }
        let schemas: Vec<&str> = b.contract.schemas.keys().map(String::as_str).collect();
        assert_eq!(schemas, ["Bookmark"]);
    }

    /// S10: A None return is 204 unless the tag says otherwise
    #[test]
    fn s10_none_return_is_204_unless_tag_says_otherwise() {
        let source = r##"from typing import Protocol


# @hexa:api service=items
class Items(Protocol):
    # @hexa:api DELETE /items/{id}
    def remove(self, id: str) -> None:
        ...

    # @hexa:api POST /items/{id}/start 202
    def start(self, id: str) -> None:
        ...
"##;
        let b = build_py(source);
        assert!(b.errors.is_empty(), "{:?}", b.errors);
        let remove = op(&b, "remove");
        assert_eq!(remove.success, 204);
        assert_eq!(remove.response, None);
        let start = op(&b, "start");
        assert_eq!(start.success, 202);
        assert_eq!(start.response, None);
    }
}

mod part_2 {
    use hexa_analysis::api_contract::{build, ApiBuild, FileApi};
    use hexa_analysis::domain::{
        ApiBody, ApiOperation, ApiParam, ApiSchema, ErrorChannel, HexLayer, Language, TypeRef,
    };
    use hexa_analysis::treesitter_api::extract;

    const PORT_FILE: &str = "src/ports/bookmarks.py";

    fn file_api(file: &str, layer: HexLayer, source: &str) -> FileApi {
        let extracted = extract(source, Language::Python);
        assert!(
            extracted.is_ok(),
            "extract failed: {:?}\n{source}",
            extracted.as_ref().err()
        );
        FileApi {
            file: file.to_string(),
            layer,
            facts: extracted.expect("checked just above"),
        }
    }

    fn build_port(source: &str) -> ApiBuild {
        build(&[file_api(PORT_FILE, HexLayer::Ports, source)])
    }

    fn build_clean(source: &str) -> ApiBuild {
        let built = build_port(source);
        assert!(
            built.errors.is_empty(),
            "unexpected errors: {:?}\n{source}",
            built.errors
        );
        built
    }

    fn op<'a>(built: &'a ApiBuild, method: &str) -> &'a ApiOperation {
        let found = built
            .contract
            .operations
            .iter()
            .find(|o| o.method_name == method);
        assert!(
            found.is_some(),
            "no operation for `{method}`; errors: {:?}; operations: {:?}",
            built.errors,
            built.contract.operations
        );
        found.expect("checked just above")
    }

    fn param<'a>(operation: &'a ApiOperation, name: &str) -> &'a ApiParam {
        let found = operation.params.iter().find(|p| p.name == name);
        assert!(
            found.is_some(),
            "no parameter `{name}`: {:?}",
            operation.params
        );
        found.expect("checked just above")
    }

    /// The 1-based number of the only line holding `needle`.
    fn line_of(source: &str, needle: &str) -> usize {
        let found: Vec<usize> = source
            .lines()
            .enumerate()
            .filter(|(_, l)| l.contains(needle))
            .map(|(i, _)| i + 1)
            .collect();
        assert_eq!(found.len(), 1, "`{needle}` must be on exactly one line");
        found[0]
    }

    fn assert_error_at(built: &ApiBuild, lines: &[usize], source: &str) {
        assert!(
            built
                .errors
                .iter()
                .any(|d| d.file == PORT_FILE && lines.contains(&d.line)),
            "expected an error at {PORT_FILE} line {lines:?}, got {:?}\n{source}",
            built.errors
        );
    }

    /// A one-method tagged port whose method is written on one line.
    fn one_line_port(decorator: Option<&str>, def_line: &str) -> String {
        let mut s = String::from(
        "from typing import Protocol\n\n\n# @hexa:api service=bookmarks\nclass Bookmarks(Protocol):\n    # @hexa:api GET /find\n",
    );
        if let Some(d) = decorator {
            s.push_str("    ");
            s.push_str(d);
            s.push('\n');
        }
        s.push_str("    ");
        s.push_str(def_line);
        s.push('\n');
        s
    }

    /// S11: String annotations and the future import do not change the reading
    #[test]
    fn s11_string_annotations_and_future_import_do_not_change_the_reading() {
        fn source(quoted: bool, future: bool) -> String {
            let q = |t: &str| {
                if quoted {
                    format!("\"{t}\"")
                } else {
                    t.to_string()
                }
            };
            let mut s = String::new();
            if future {
                s.push_str("from __future__ import annotations\n");
            }
            s.push_str("from dataclasses import dataclass\nfrom typing import Protocol\n\n\n");
            s.push_str(&format!(
            "@dataclass\nclass Bookmark:\n    url: {}\n    title: {}\n    tags: {}\n    note: {} = None\n\n\n",
            q("str"),
            q("str"),
            q("list[str]"),
            q("str | None")
        ));
            s.push_str("# @hexa:api service=bookmarks\nclass Bookmarks(Protocol):\n");
            s.push_str(&format!(
                "    # @hexa:api POST /bookmarks\n    def save(self, bookmark: {}) -> {}: ...\n\n",
                q("Bookmark"),
                q("Bookmark")
            ));
            s.push_str(&format!(
            "    # @hexa:api GET /bookmarks\n    def search(self, tag: {} = None) -> {}: ...\n\n",
            q("str | None"),
            q("list[Bookmark]")
        ));
            s.push_str(&format!(
            "    # @hexa:api DELETE /bookmarks/{{ident}}\n    def remove(self, ident: {}) -> {}: ...\n",
            q("str"),
            q("None")
        ));
            s
        }

        fn normalized(built: &ApiBuild) -> Vec<ApiOperation> {
            let mut ops = built.contract.operations.clone();
            for o in &mut ops {
                o.line = 0;
                for a in &mut o.args {
                    a.written = String::new();
                }
            }
            ops.sort_by(|a, b| a.method_name.cmp(&b.method_name));
            ops
        }

        let plain = build_clean(&source(false, false));
        let strings = build_clean(&source(true, false));
        let future = build_clean(&source(false, true));

        for built in [&plain, &strings, &future] {
            let mut names: Vec<&str> = built
                .contract
                .operations
                .iter()
                .map(|o| o.method_name.as_str())
                .collect();
            names.sort_unstable();
            assert_eq!(names, vec!["remove", "save", "search"]);
            assert!(built.contract.schemas.contains_key("Bookmark"));
        }
        assert_eq!(normalized(&plain), normalized(&strings));
        assert_eq!(normalized(&plain), normalized(&future));
        assert_eq!(plain.contract.schemas, strings.contract.schemas);
        assert_eq!(plain.contract.schemas, future.contract.schemas);
    }

    /// S12: Only a nullable type with a None default is made optional by its default
    #[test]
    fn s12_only_a_nullable_type_with_a_none_default_is_optional() {
        let source = one_line_port(
        None,
        "def find(self, a: str = \"x\", b: int = 5, c: str = None, d: str | None = None) -> str: ...",
    );
        let built = build_clean(&source);
        let find = op(&built, "find");
        assert_eq!(find.params.len(), 4, "{:?}", find.params);
        assert!(param(find, "a").required);
        assert!(param(find, "b").required);
        assert!(param(find, "c").required);
        assert!(!param(find, "d").required);
        assert_eq!(param(find, "d").ty, TypeRef::String);
    }

    /// S13: Unannotated and variadic signatures are refused
    #[test]
    fn s13_unannotated_and_variadic_signatures_are_refused() {
        let control = one_line_port(None, "def find(self, q: str) -> str: ...");
        assert_eq!(build_clean(&control).contract.operations.len(), 1);

        let cases: [(Option<&str>, &str); 6] = [
            (None, "def find(self, q) -> str: ..."),
            (None, "def find(self, q: str): ..."),
            (None, "def find(self, q=\"x\") -> str: ..."),
            (Some("@staticmethod"), "def find(q) -> str: ..."),
            (None, "def find(self, *args: str) -> str: ..."),
            (None, "def find(self, **kwargs: str) -> str: ..."),
        ];
        for (decorator, def_line) in cases {
            let source = one_line_port(decorator, def_line);
            let def_at = line_of(&source, def_line);
            let mut lines = vec![def_at];
            if decorator.is_some() {
                lines.push(def_at - 1);
            }
            let built = build_port(&source);
            assert_error_at(&built, &lines, &source);
        }
    }

    /// S14: A string annotation that does not parse is an error
    #[test]
    fn s14_a_string_annotation_that_does_not_parse_is_an_error() {
        for def_line in [
            "def find(self, q: \"list[\") -> str: ...",
            "def find(self, q: str) -> \"list[\": ...",
        ] {
            let source = one_line_port(None, def_line);
            let built = build_port(&source);
            assert_error_at(&built, &[line_of(&source, def_line)], &source);
            for operation in &built.contract.operations {
                // An operation kept beside the error must not carry a made-up type.
                if def_line.contains("-> \"list[\"") {
                    assert!(
                        matches!(operation.response, Some(TypeRef::Unsupported(_)) | None),
                        "a type was substituted for the return: {operation:?}"
                    );
                } else {
                    assert!(
                        operation
                            .params
                            .iter()
                            .filter(|p| p.name == "q")
                            .all(|p| matches!(p.ty, TypeRef::Unsupported(_))),
                        "a type was substituted for the parameter: {operation:?}"
                    );
                }
            }
        }
    }

    /// S15: Spelling of the signature does not change the operation
    #[test]
    fn s15_spelling_of_the_signature_does_not_change_the_operation() {
        struct Signature {
            tag: &'static str,
            params: &'static [(&'static str, &'static str, &'static str)],
            returns: &'static str,
        }

        #[derive(Clone, Copy)]
        struct Spelling {
            receiver: &'static str,
            slash_after: Option<usize>,
            star: bool,
            is_async: bool,
            wrap: usize,
            quote: usize,
            future: bool,
        }

        fn render(sig: &Signature, sp: Spelling) -> String {
            let quote = |t: &str| format!("\"{t}\"");
            let mut parts = vec![sp.receiver.to_string()];
            if sp.slash_after == Some(0) {
                parts.push("/".to_string());
            }
            for (i, (name, ty, default)) in sig.params.iter().enumerate() {
                if sp.star && i + 1 == sig.params.len() {
                    parts.push("*".to_string());
                }
                let ty = if sp.quote == 1 || sp.quote == 4 {
                    quote(ty)
                } else {
                    ty.to_string()
                };
                parts.push(format!("{name}: {ty}{default}"));
                if sp.slash_after == Some(i + 1) {
                    parts.push("/".to_string());
                }
            }
            // quote 3 quotes only the type inside a wrapper.
            let inner = if sp.quote == 3 {
                quote(sig.returns)
            } else {
                sig.returns.to_string()
            };
            let mut returns = match sp.wrap {
                0 => inner,
                1 => format!("Awaitable[{inner}]"),
                _ => format!("Coroutine[Any, Any, {inner}]"),
            };
            if sp.quote == 2 || sp.quote == 4 {
                returns = quote(&returns);
            }
            let mut s = String::new();
            if sp.future {
                s.push_str("from __future__ import annotations\n");
            }
            s.push_str("from dataclasses import dataclass\nfrom typing import Any, Awaitable, Coroutine, Protocol\n\n\n");
            s.push_str("@dataclass\nclass Item:\n    title: str\n    count: int\n\n\n");
            s.push_str("# @hexa:api service=items\nclass Items(Protocol):\n");
            s.push_str(&format!(
                "    # @hexa:api {}\n    {}def handle({}) -> {}: ...\n",
                sig.tag,
                if sp.is_async { "async " } else { "" },
                parts.join(", "),
                returns
            ));
            s
        }

        type View = (
            Vec<ApiParam>,
            Option<ApiBody>,
            u16,
            Option<TypeRef>,
            Vec<u16>,
            std::collections::BTreeMap<String, ApiSchema>,
        );

        fn view(source: &str) -> View {
            let built = build_clean(source);
            assert_eq!(built.contract.operations.len(), 1, "{source}");
            let o = op(&built, "handle");
            (
                o.params.clone(),
                o.body.clone(),
                o.success,
                o.response.clone(),
                o.errors.clone(),
                built.contract.schemas.clone(),
            )
        }

        let signatures = [
            Signature {
                tag: "GET /items/{item_id}",
                params: &[
                    ("item_id", "str", ""),
                    ("limit", "int", ""),
                    ("tag", "str | None", " = None"),
                ],
                returns: "list[Item]",
            },
            Signature {
                tag: "POST /items",
                params: &[("title", "str", ""), ("count", "int", "")],
                returns: "Item",
            },
            Signature {
                tag: "DELETE /items/{item_id}",
                params: &[("item_id", "str", "")],
                returns: "None",
            },
        ];

        for sig in &signatures {
            let base = Spelling {
                receiver: "self",
                slash_after: None,
                star: false,
                is_async: false,
                wrap: 0,
                quote: 0,
                future: false,
            };
            let expected = view(&render(sig, base));
            if sig.returns == "None" {
                assert_eq!(expected.2, 204);
                assert_eq!(expected.3, None);
            }
            for receiver in ["self", "cls", "this", "_"] {
                for slash_after in [None, Some(0), Some(1)] {
                    for star in [false, true] {
                        // `/` may not come after `*`.
                        if star && slash_after.is_some_and(|at| at >= sig.params.len()) {
                            continue;
                        }
                        for is_async in [false, true] {
                            for wrap in 0..3 {
                                for quote in 0..5 {
                                    for future in [false, true] {
                                        let source = render(
                                            sig,
                                            Spelling {
                                                receiver,
                                                slash_after,
                                                star,
                                                is_async,
                                                wrap,
                                                quote,
                                                future,
                                            },
                                        );
                                        assert_eq!(view(&source), expected, "{source}");
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// S16: Class and method status comments declare errors
    #[test]
    fn s16_class_and_method_status_comments_declare_errors() {
        let source = r#"from typing import Protocol


# Saved links.
# @hexa:api service=bookmarks
# @hexa:status 401 caller is not signed in
class Bookmarks(Protocol):
    # Fetch one link.
    # @hexa:api GET /bookmarks/{ident}
    # @hexa:status 404 no such bookmark
    def get(self, ident: str) -> str: ...

    # Count the links.
    # @hexa:api GET /count
    def count(self) -> int: ...
"#;
        let built = build_clean(source);
        assert_eq!(built.contract.operations.len(), 2);

        let get = op(&built, "get");
        let count = op(&built, "count");
        for code in [401, 404, 500] {
            assert!(get.errors.contains(&code), "{:?}", get.errors);
        }
        let mut sorted = get.errors.clone();
        sorted.sort_unstable();
        assert_eq!(get.errors, sorted);
        assert!(count.errors.contains(&401), "{:?}", count.errors);
        assert!(count.errors.contains(&500), "{:?}", count.errors);
        assert!(!count.errors.contains(&404), "{:?}", count.errors);

        for o in &built.contract.operations {
            for leaked in ["@hexa:", "caller is not signed in", "no such bookmark"] {
                assert!(!o.description.contains(leaked), "{:?}", o.description);
            }
        }
        assert!(get.description.contains("Fetch one link."));
        assert!(count.description.contains("Count the links."));
    }

    /// S17: Every Python method has an opaque error channel
    #[test]
    fn s17_every_python_method_has_an_opaque_error_channel() {
        let source = one_line_port(None, "def find(self) -> str: ...");
        let file = file_api(PORT_FILE, HexLayer::Ports, &source);
        let method = file
            .facts
            .ports
            .iter()
            .flat_map(|p| p.methods.iter())
            .find(|m| m.name == "find");
        assert!(method.is_some(), "method not extracted: {:?}", file.facts);
        assert_eq!(
            method.expect("checked just above").error,
            ErrorChannel::Opaque
        );

        let built = build(&[file]);
        assert!(built.errors.is_empty(), "{:?}", built.errors);
        let find = op(&built, "find");
        assert!(find.fails);
        assert!(find.errors.contains(&500), "{:?}", find.errors);
        assert_eq!(find.error_type, None);
        assert!(find.error_variants.is_empty());
    }

    /// S18: Docstrings, raised exceptions and exception classes declare no errors
    #[test]
    fn s18_docstrings_raises_and_exception_classes_declare_no_errors() {
        let exceptions = r#"# @hexa:status 410 gone for good
class NotFound(Exception):
    pass


# @hexa:status 409 it clashes
class Conflict(Exception):
    pass
"#;
        let port = r#"from typing import Protocol

from domain.errors import Conflict, NotFound


# @hexa:api service=bookmarks
class Bookmarks(Protocol):
    # Fetch one link.
    # @hexa:api GET /bookmarks/{ident}
    def get(self, ident: str) -> str:
        """Fetch one link.

        @hexa:status 404 gone

        Raises:
            NotFound: 404 when there is no such link.
            Conflict: 409 when it clashes.
        """
        raise NotFound("404")
"#;
        let built = build(&[
            file_api("src/domain/errors.py", HexLayer::Domain, exceptions),
            file_api(PORT_FILE, HexLayer::Ports, port),
        ]);
        assert!(built.errors.is_empty(), "{:?}", built.errors);
        assert_eq!(built.contract.operations.len(), 1);
        let get = op(&built, "get");
        assert!(get.fails);
        assert_eq!(get.errors, vec![500]);
        assert_eq!(get.error_type, None);
        assert!(get.error_variants.is_empty());
    }

    /// S19: Declared errors follow their scope
    #[test]
    fn s19_declared_errors_follow_their_scope() {
        const CODES: [u16; 8] = [400, 401, 403, 404, 409, 422, 429, 503];

        fn next(state: &mut u64) -> u64 {
            *state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            *state >> 33
        }

        fn subset(state: &mut u64) -> Vec<u16> {
            // Each code is chosen one time in four.
            CODES
                .iter()
                .copied()
                .filter(|_| next(state) % 4 == 0)
                .collect()
        }

        let mut state = 0x5eed_u64;
        for case in 0..200 {
            let method_count = 1 + case % 4;
            let class_codes = subset(&mut state);
            let method_codes: Vec<Vec<u16>> =
                (0..method_count).map(|_| subset(&mut state)).collect();

            let mut source = String::from("from typing import Protocol\n\n\n# @hexa:api service=things\n");
            for code in &class_codes {
                source.push_str(&format!("# @hexa:status {code} class wide\n"));
            }
            source.push_str("class Things(Protocol):\n");
            for (i, codes) in method_codes.iter().enumerate() {
                let tag = format!("    # @hexa:api GET /thing{i}\n");
                let tag_first = next(&mut state) % 2 == 0;
                if tag_first {
                    source.push_str(&tag);
                }
                for code in codes {
                    source.push_str(&format!("    # @hexa:status {code} own error\n"));
                }
                if !tag_first {
                    source.push_str(&tag);
                }
                source.push_str(&format!("    def method{i}(self) -> str: ...\n\n"));
            }

            let built = build_clean(&source);
            assert_eq!(built.contract.operations.len(), method_count, "{source}");
            for (i, own) in method_codes.iter().enumerate() {
                let o = op(&built, &format!("method{i}"));
                assert!(o.errors.contains(&500), "{:?}\n{source}", o.errors);
                for code in class_codes.iter().chain(own.iter()) {
                    assert!(o.errors.contains(code), "missing {code}\n{source}");
                }
                for code in CODES {
                    if !class_codes.contains(&code) && !own.contains(&code) {
                        assert!(!o.errors.contains(&code), "leaked {code}\n{source}");
                    }
                }
                assert!(
                    o.errors.windows(2).all(|w| w[0] <= w[1]),
                    "unsorted: {:?}",
                    o.errors
                );
            }
        }
    }

    /// S20: The three schema class kinds keep field names as written
    #[test]
    fn s20_three_schema_class_kinds_keep_field_names_as_written() {
        let source = r#"from dataclasses import dataclass
from typing import Protocol, TypedDict

from pydantic import BaseModel


@dataclass
class Saved:
    saved_at: str
    pageURL: int
    Score_value: float
    is_read: bool


class Listed(TypedDict):
    pageURL: str
    saved_at: int
    avgRank: float
    HasMore: bool


class Modelled(BaseModel):
    zeta: bool
    saved_at: float
    pageURL: str
    visit_Count: int


# @hexa:api service=bookmarks
class Bookmarks(Protocol):
    # @hexa:api GET /saved
    def saved(self) -> Saved: ...

    # @hexa:api GET /listed
    def listed(self) -> Listed: ...

    # @hexa:api GET /modelled
    def modelled(self) -> Modelled: ...
"#;
        let built = build_clean(source);
        assert_eq!(built.contract.operations.len(), 3);

        let object = |fields: &[(&str, TypeRef)]| {
            ApiSchema::Object(
                fields
                    .iter()
                    .map(|(n, t)| (n.to_string(), t.clone(), true))
                    .collect(),
            )
        };
        assert_eq!(
            built.contract.schemas.get("Saved"),
            Some(&object(&[
                ("saved_at", TypeRef::String),
                ("pageURL", TypeRef::Integer),
                ("Score_value", TypeRef::Number),
                ("is_read", TypeRef::Boolean),
            ]))
        );
        assert_eq!(
            built.contract.schemas.get("Listed"),
            Some(&object(&[
                ("pageURL", TypeRef::String),
                ("saved_at", TypeRef::Integer),
                ("avgRank", TypeRef::Number),
                ("HasMore", TypeRef::Boolean),
            ]))
        );
        assert_eq!(
            built.contract.schemas.get("Modelled"),
            Some(&object(&[
                ("zeta", TypeRef::Boolean),
                ("saved_at", TypeRef::Number),
                ("pageURL", TypeRef::String),
                ("visit_Count", TypeRef::Integer),
            ]))
        );
    }
}

mod part_3 {
    use std::path::Path;
    use std::process::Command;

    use hexa_analysis::api_contract::{build, ApiBuild, FileApi};
    use hexa_analysis::domain::{ApiSchema, HexLayer, Language, TypeRef};
    use hexa_analysis::treesitter_api::extract;

    const PRELUDE: &str = "\
import typing
from typing import Any, Annotated, ClassVar, Dict, List, Literal, Mapping, NamedTuple, NewType, Optional, Protocol, Sequence, Tuple, TypedDict, Union
from typing import NotRequired, Required
from dataclasses import dataclass, field, InitVar, KW_ONLY
from pydantic import BaseModel, ConfigDict, Field, computed_field, field_serializer, model_serializer
from datetime import datetime, date
from uuid import UUID
from decimal import Decimal
from enum import Enum
";

    /// The text after `@hexa:api` on a port, found by asking the Rust reader,
    /// which the contract already supports, which form it accepts.
    fn port_tag() -> &'static str {
        for candidate in ["", " bookmarks", " service=bookmarks", " /"] {
            let source = format!(
            "/// @hexa:api{candidate}\npub trait Probe {{\n    /// @hexa:api GET /probe\n    fn probe(&self) -> String;\n}}\n"
        );
            let Ok(facts) = extract(&source, Language::Rust) else {
                continue;
            };
            let built = build(&[FileApi {
                file: "src/ports/probe.rs".to_string(),
                layer: HexLayer::Ports,
                facts,
            }]);
            if built.errors.is_empty() && built.contract.operations.len() == 1 {
                return candidate;
            }
        }
        ""
    }

    /// A tagged port whose methods are `(verb and path, signature)`.
    fn port(tag: &str, methods: &[(&str, &str)]) -> String {
        let mut out = format!("\n# @hexa:api{tag}\nclass Things(Protocol):\n");
        for (route, signature) in methods {
            out.push_str(&format!(
                "    # @hexa:api {route}\n    {signature}: ...\n\n"
            ));
        }
        out
    }

    fn build_py(files: &[(&str, &str)]) -> ApiBuild {
        let files: Vec<FileApi> = files
            .iter()
            .map(|(file, source)| FileApi {
                file: file.to_string(),
                layer: if file.contains("ports/") {
                    HexLayer::Ports
                } else {
                    HexLayer::Domain
                },
                facts: extract(source, Language::Python).expect("Python source is read"),
            })
            .collect();
        build(&files)
    }

    /// The 1-based line of the first line containing `needle`.
    fn line_of(source: &str, needle: &str) -> usize {
        source
            .lines()
            .position(|l| l.contains(needle))
            .map(|i| i + 1)
            .unwrap_or_else(|| panic!("no line contains {needle:?} in:\n{source}"))
    }

    fn assert_clean(built: &ApiBuild) {
        assert!(
            built.errors.is_empty(),
            "unexpected errors: {:?}",
            built.errors
        );
    }

    fn object<'a>(built: &'a ApiBuild, name: &str) -> &'a [(String, TypeRef, bool)] {
        match built.contract.schemas.get(name) {
            Some(ApiSchema::Object(fields)) => fields,
            other => panic!(
                "expected object schema {name}, got {other:?}; schemas: {:?}; errors: {:?}",
                built.contract.schemas, built.errors
            ),
        }
    }

    fn field_of(built: &ApiBuild, schema: &str, name: &str) -> (TypeRef, bool) {
        object(built, schema)
            .iter()
            .find(|f| f.0 == name)
            .map(|f| (f.1.clone(), f.2))
            .unwrap_or_else(|| panic!("schema {schema} has no field {name}"))
    }

    fn leaf(ty: &TypeRef) -> &TypeRef {
        match ty {
            TypeRef::Array(t) | TypeRef::Map(t) | TypeRef::Optional(t) => leaf(t),
            other => other,
        }
    }

    fn is_guess(ty: &TypeRef) -> bool {
        matches!(
            leaf(ty),
            TypeRef::String | TypeRef::Integer | TypeRef::Number | TypeRef::Boolean
        )
    }

    fn assert_no_empty_schema(built: &ApiBuild, context: &str) {
        for (name, schema) in &built.contract.schemas {
            let empty = match schema {
                ApiSchema::Object(f) => f.is_empty(),
                ApiSchema::StringEnum(v) => v.is_empty(),
            };
            assert!(!empty, "{context}: empty schema published for {name}");
        }
    }

    /// S21: Containers, NewType and names across files resolve
    #[test]
    fn s21_containers_newtype_and_cross_file_names_resolve() {
        let models = format!("{PRELUDE}\n@dataclass\nclass Money:\n    amount: int\n");
        let shapes = "\
from domain.m import Money as Cash
from domain import m

UserId = NewType(\"UserId\", str)

@dataclass
class Shapes:
    a: list[int]
    b: Sequence[int]
    c: tuple[int, ...]
    d: List[int]
    e: typing.List[int]
    f: typing.Sequence[int]
    g: Tuple[int, ...]
    h: typing.Tuple[int, ...]
    i: dict[str, int]
    j: Mapping[str, int]
    k: Dict[str, int]
    l: typing.Dict[str, int]
    n: typing.Mapping[str, int]
    uid: UserId
    aliased: Cash
    dotted: m.Money
    many: list[Cash]
    keyed: dict[str, m.Money]
";
        let api = format!(
            "{PRELUDE}{shapes}{}",
            port(
                port_tag(),
                &[("GET /shapes", "def get_shapes(self) -> Shapes")]
            )
        );
        let built = build_py(&[("domain/m.py", &models), ("ports/api.py", &api)]);
        assert_clean(&built);

        let array = TypeRef::Array(Box::new(TypeRef::Integer));
        for name in ["a", "b", "c", "d", "e", "f", "g", "h"] {
            assert_eq!(field_of(&built, "Shapes", name).0, array, "field {name}");
        }
        let map = TypeRef::Map(Box::new(TypeRef::Integer));
        for name in ["i", "j", "k", "l", "n"] {
            assert_eq!(field_of(&built, "Shapes", name).0, map, "field {name}");
        }
        assert_eq!(field_of(&built, "Shapes", "uid").0, TypeRef::String);
        let money = TypeRef::Named("Money".to_string());
        assert_eq!(field_of(&built, "Shapes", "aliased").0, money);
        assert_eq!(field_of(&built, "Shapes", "dotted").0, money);
        assert_eq!(
            field_of(&built, "Shapes", "many").0,
            TypeRef::Array(Box::new(money.clone()))
        );
        assert_eq!(
            field_of(&built, "Shapes", "keyed").0,
            TypeRef::Map(Box::new(money))
        );
        assert_eq!(
            object(&built, "Money"),
            &[("amount".to_string(), TypeRef::Integer, true)][..]
        );
        assert!(!built.contract.schemas.contains_key("Cash"));
    }

    /// S22: Requiredness follows the rules for each schema kind
    #[test]
    fn s22_requiredness_follows_each_schema_kind() {
        let classes = "
@dataclass
class Data:
    c: int
    a: str | None = None
    b: str = \"x\"

class Model(BaseModel):
    a: str | None = None
    b: str = \"x\"
    c: int

class Total(TypedDict):
    d: NotRequired[str]
    e: str

class Partial(TypedDict, total=False):
    f: str
    g: Required[str]
";
        let api = format!(
            "{PRELUDE}{classes}{}",
            port(
                port_tag(),
                &[
                    ("GET /data", "def get_data(self) -> Data"),
                    ("GET /model", "def get_model(self) -> Model"),
                    ("GET /total", "def get_total(self) -> Total"),
                    ("GET /partial", "def get_partial(self) -> Partial"),
                ]
            )
        );
        let built = build_py(&[("ports/api.py", &api)]);
        assert_clean(&built);

        for schema in ["Data", "Model"] {
            assert_eq!(
                field_of(&built, schema, "a"),
                (TypeRef::String, false),
                "{schema}.a"
            );
            assert_eq!(
                field_of(&built, schema, "b"),
                (TypeRef::String, true),
                "{schema}.b"
            );
            assert_eq!(
                field_of(&built, schema, "c"),
                (TypeRef::Integer, true),
                "{schema}.c"
            );
        }
        assert_eq!(field_of(&built, "Total", "d"), (TypeRef::String, false));
        assert_eq!(field_of(&built, "Total", "e"), (TypeRef::String, true));
        assert_eq!(field_of(&built, "Partial", "f"), (TypeRef::String, false));
        assert_eq!(field_of(&built, "Partial", "g"), (TypeRef::String, true));
    }

    /// S23: Permitted field values and config are accepted, and non-fields are omitted
    #[test]
    fn s23_permitted_values_and_config_are_accepted_and_non_fields_omitted() {
        let classes = "
@dataclass(frozen=True)
class Data:
    shared: ClassVar[int] = 0
    _hidden: typing.ClassVar[str] = \"h\"
    seed: InitVar[int] = 0
    _salt: InitVar[str] = \"s\"
    _: KW_ONLY
    text: str = \"x\"
    count: int = 3
    ratio: float = 1.5
    flag: bool = True
    items: list[str] = field(default_factory=list)
    word: str = field(default=\"y\")
    maybe: str | None = None

class Model(BaseModel):
    model_config = ConfigDict(frozen=True)
    counter: ClassVar[int] = 0
    title: str = Field(default=\"x\", description=\"d\")
    tags: list[str] = Field(default_factory=list)
    size: int = 1

class Legacy(BaseModel):
    class Config:
        frozen = True

    name: str

class Annotation(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True)
    value: str
";
        let api = format!(
            "{PRELUDE}{classes}{}",
            port(
                port_tag(),
                &[
                    ("GET /data", "def get_data(self) -> Data"),
                    ("GET /model", "def get_model(self) -> Model"),
                    ("GET /legacy", "def get_legacy(self) -> Legacy"),
                    ("GET /annotation", "def get_annotation(self) -> Annotation"),
                ]
            )
        );
        let built = build_py(&[("ports/api.py", &api)]);
        assert_clean(&built);

        let expected: [(&str, &[&str]); 4] = [
            (
                "Data",
                &["text", "count", "ratio", "flag", "items", "word", "maybe"],
            ),
            ("Model", &["title", "tags", "size"]),
            ("Legacy", &["name"]),
            ("Annotation", &["value"]),
        ];
        for (schema, names) in expected {
            let got: Vec<&str> = object(&built, schema)
                .iter()
                .map(|f| f.0.as_str())
                .collect();
            assert_eq!(got, names, "fields of {schema}");
        }
    }

    const REFUSED_DECLS: &str = "
class Pair(NamedTuple):
    left: str
    right: int

class Color(Enum):
    RED = \"red\"

class Plain:
    a: str
";

    const REFUSED_TYPES: &[&str] = &[
        "Pair",
        "Any",
        "str | int",
        "tuple[str, int]",
        "list",
        "dict",
        "Color",
        "Literal[\"a\"]",
        "datetime",
        "date",
        "UUID",
        "Decimal",
        "set",
        "bytes",
        "Annotated[str, \"x\"]",
        "Plain",
    ];

    /// S24: Unsupported types are errors and never empty schemas
    #[test]
    fn s24_unsupported_types_are_errors_never_empty_schemas() {
        let tag = port_tag();
        for refused in REFUSED_TYPES {
            let api = format!(
                "{PRELUDE}{REFUSED_DECLS}\n@dataclass\nclass Holder:\n    value: {refused}\n{}",
                port(tag, &[("GET /holder", "def get_holder(self) -> Holder")])
            );
            let line = line_of(&api, "    value: ");
            let built = build_py(&[("ports/api.py", &api)]);
            assert!(
                built
                    .errors
                    .iter()
                    .any(|e| e.file == "ports/api.py" && e.line == line),
                "{refused}: expected an error at line {line}, got {:?}",
                built.errors
            );
            assert_no_empty_schema(&built, refused);
            if let Some(ApiSchema::Object(fields)) = built.contract.schemas.get("Holder") {
                for (name, ty, _) in fields {
                    assert!(
                        !(name == "value" && is_guess(ty)),
                        "{refused}: substitute type {ty:?} published"
                    );
                }
            }
        }
    }

    /// S25: A required field that allows None is refused
    #[test]
    fn s25_required_nullable_field_is_refused() {
        let tag = port_tag();
        let make = |class: &str| {
            format!(
                "{PRELUDE}\n{class}\n{}",
                port(tag, &[("GET /holder", "def get_holder(self) -> Holder")])
            )
        };
        let cases = [
            "@dataclass\nclass Holder:\n    value: str | None\n",
            "class Holder(BaseModel):\n    value: Optional[str] = \"x\"\n",
            "class Holder(TypedDict):\n    value: str | None\n",
        ];
        for class in cases {
            let api = make(class);
            let built = build_py(&[("ports/api.py", &api)]);
            assert!(
                built.errors.iter().any(|e| e.file == "ports/api.py"),
                "expected an error for:\n{class}\ngot {:?}",
                built.errors
            );
        }
        // The control: the same field, allowed to be absent, is accepted.
        let control = make("@dataclass\nclass Holder:\n    value: str | None = None\n");
        let built = build_py(&[("ports/api.py", &control)]);
        assert_clean(&built);
        assert_eq!(
            field_of(&built, "Holder", "value"),
            (TypeRef::String, false)
        );
    }

    /// S26: Class features that could change what is sent are refused
    #[test]
    fn s26_class_features_that_change_the_wire_are_refused() {
        let cases = [
        "@dataclass\nclass Base:\n    z: int\n\n@dataclass\nclass Refused(Base):\n    a: int\n",
        "class Base(BaseModel):\n    z: int\n\nclass Refused(Base):\n    a: int\n",
        "class Base(TypedDict):\n    z: int\n\nclass Refused(Base):\n    a: int\n",
        "class Refused(BaseModel, SomeMixin):\n    a: int\n",
        "@total_ordering\n@dataclass\nclass Refused:\n    a: int\n",
        "class Refused(BaseModel):\n    a: int\n\n    @computed_field\n    @property\n    def b(self) -> int:\n        return 1\n",
        "class Refused(BaseModel):\n    a: int\n\n    @field_serializer(\"a\")\n    def ser(self, v: int) -> str:\n        return str(v)\n",
        "class Refused(BaseModel):\n    a: int\n\n    @model_serializer\n    def ser(self) -> str:\n        return \"x\"\n",
        "class Refused(BaseModel):\n    model_config = ConfigDict(alias_generator=str.upper)\n    a: int\n",
        "class Refused(BaseModel):\n    model_config = ConfigDict(frozen=True, extra=\"allow\")\n    a: int\n",
        "class Refused(BaseModel):\n    class Config:\n        extra = \"allow\"\n\n    a: int\n",
        "@dataclass(config=ConfigDict(frozen=True))\nclass Refused:\n    a: int\n",
        "class Refused(BaseModel, alias_generator=str.upper):\n    a: int\n",
        "class Refused(TypedDict, closed=True):\n    a: int\n",
    ];
        let tag = port_tag();
        for class in cases {
            let models = format!("{PRELUDE}\n{class}");
            let api = format!(
                "{PRELUDE}from domain.models import Refused\n{}",
                port(tag, &[("GET /thing", "def get_thing(self) -> Refused")])
            );
            let class_line = line_of(&models, "class Refused");
            // The declaration begins at its first decorator, if it has one.
            let lines: Vec<&str> = models.lines().collect();
            let mut first = class_line;
            while first > 1 && lines[first - 2].starts_with('@') {
                first -= 1;
            }
            let built = build_py(&[("domain/models.py", &models), ("ports/api.py", &api)]);
            assert!(
            built.errors.iter().any(|e| e.file == "domain/models.py"
                && (first..=class_line).contains(&e.line)),
            "expected an error at domain/models.py:{class_line} for:\n{class}\ngot {:?}",
            built.errors
        );
            assert_no_empty_schema(&built, class);
            if let Some(ApiSchema::Object(fields)) = built.contract.schemas.get("Refused") {
                assert!(
                    fields.iter().all(|f| !is_guess(&f.1)),
                    "guessed schema published for:\n{class}\n{fields:?}"
                );
            }
        }
    }

    /// S27: Private fields and unreadable field values are refused
    #[test]
    fn s27_private_fields_and_unreadable_values_are_refused() {
        let cases = [
            ("_secret", "@dataclass\nclass Refused:\n    _secret: str\n"),
            (
                "bad",
                "@dataclass\nclass Refused:\n    bad: str = make_default()\n",
            ),
            (
                "bad",
                "class Refused(BaseModel):\n    bad: str = Field(alias=\"other\")\n",
            ),
            (
                "bad",
                "class Refused(BaseModel):\n    bad: str = Field(exclude=True)\n",
            ),
            (
                "bad",
                "@dataclass\nclass Refused:\n    bad: str = field(init=False)\n",
            ),
        ];
        let tag = port_tag();
        for (name, class) in cases {
            let models = format!("{PRELUDE}\n{class}");
            let api = format!(
                "{PRELUDE}from domain.models import Refused\n{}",
                port(tag, &[("GET /thing", "def get_thing(self) -> Refused")])
            );
            let field_line = line_of(&models, &format!("    {name}: str"));
            let use_line = line_of(&api, "def get_thing");
            let built = build_py(&[("domain/models.py", &models), ("ports/api.py", &api)]);
            assert!(
                built.errors.iter().any(|e| {
                    (e.file == "ports/api.py" && e.line == use_line)
                        || (e.file == "domain/models.py" && e.line == field_line)
                }),
                "expected an error where the type is used for:\n{class}\ngot {:?}",
                built.errors
            );
            if let Some(ApiSchema::Object(fields)) = built.contract.schemas.get("Refused") {
                assert!(
                    fields
                        .iter()
                        .all(|f| f.0 != name || matches!(leaf(&f.1), TypeRef::Unsupported(_))),
                    "{name} published as a valid field: {fields:?}"
                );
            }
        }
    }

    fn write(root: &Path, file: &str, text: &str) {
        let path = root.join(file);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("create dirs");
        std::fs::write(path, text).expect("write file");
    }

    fn spec(root: &Path) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_hexa"))
            .args(["api", "spec"])
            .arg(root)
            .args(["--out", "-"])
            .output()
            .expect("run hexa api spec")
    }

    /// S28: Declarations in .pyi files are not read for the API
    #[test]
    fn s28_pyi_declarations_are_not_read() {
        let api = format!(
            "{PRELUDE}from src.domain.shapes import Thing\n{}",
            port(
                port_tag(),
                &[("GET /thing", "def get_thing(self) -> Thing")]
            )
        );
        let declaration =
            format!("{PRELUDE}\n@dataclass\nclass Thing:\n    only_declared_here: str\n");
        let project = |declared_in: &str| {
            let dir = tempfile::tempdir().expect("tempdir");
            write(
                dir.path(),
                "pyproject.toml",
                "[project]\nname = \"things\"\nversion = \"0.1.0\"\n",
            );
            write(dir.path(), "src/ports/api.py", &api);
            write(dir.path(), declared_in, &declaration);
            dir
        };

        // The control: declared in a `.py` file, the type is read.
        let control = project("src/domain/shapes.py");
        let out = spec(control.path());
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success() && stdout.contains("only_declared_here"),
            "control failed: {stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );

        let stub = project("src/domain/shapes.pyi");
        let out = spec(stub.path());
        let stdout = String::from_utf8_lossy(&out.stdout);
        let all = format!("{stdout}\n{}", String::from_utf8_lossy(&out.stderr));
        assert!(
            !out.status.success(),
            "a stub-only type was accepted: {all}"
        );
        let at = format!("src/ports/api.py:{}", line_of(&api, "def get_thing"));
        assert!(all.contains(&at), "expected an error at {at}: {all}");
        assert!(
            !stdout.contains("only_declared_here"),
            "a schema was built from the .pyi file: {stdout}"
        );
    }

    /// S29: Equivalent nullable spellings mean the same thing
    #[test]
    fn s29_nullable_spellings_are_equivalent() {
        let tag = port_tag();
        let leaf_class = "\n@dataclass\nclass Leaf:\n    z: int\n";
        let scalars = ["str", "int", "float", "bool"];
        let all = [
            "str",
            "int",
            "float",
            "bool",
            "list[str]",
            "dict[str, int]",
            "Leaf",
        ];
        let spellings = |t: &str| {
            [
                format!("{t} | None"),
                format!("None | {t}"),
                format!("Optional[{t}]"),
                format!("Union[{t}, None]"),
            ]
        };
        let positions = [
            "dataclass",
            "model",
            "typeddict",
            "typeddict-partial",
            "nested-list",
            "nested-map",
            "param",
            "return",
        ];

        for position in positions {
            let types: &[&str] = if position == "param" { &scalars } else { &all };
            for t in types {
                let variants: Vec<&str> = match position {
                    "typeddict" | "typeddict-partial" => vec!["plain", "NotRequired", "Required"],
                    "return" => vec![""],
                    _ => vec!["", " = None", " = 0"],
                };
                for variant in variants {
                    let mut first = None;
                    for spelled in spellings(t) {
                        let holder = |ann: String| {
                            format!("\n@dataclass\nclass Holder:\n    value: {ann}\n")
                        };
                        let wrapped = match variant {
                            "NotRequired" => format!("NotRequired[{spelled}]"),
                            "Required" => format!("Required[{spelled}]"),
                            _ => spelled.clone(),
                        };
                        let get_holder = "def get_holder(self) -> Holder".to_string();
                        let (class, signature) = match position {
                            "dataclass" => (holder(format!("{spelled}{variant}")), get_holder),
                            "model" => (
                                format!(
                                    "\nclass Holder(BaseModel):\n    value: {spelled}{variant}\n"
                                ),
                                get_holder,
                            ),
                            "typeddict" => (
                                format!("\nclass Holder(TypedDict):\n    value: {wrapped}\n"),
                                get_holder,
                            ),
                            "typeddict-partial" => (
                                format!(
                                "\nclass Holder(TypedDict, total=False):\n    value: {wrapped}\n"
                            ),
                                get_holder,
                            ),
                            "nested-list" => {
                                (holder(format!("list[{spelled}]{variant}")), get_holder)
                            }
                            "nested-map" => {
                                (holder(format!("dict[str, {spelled}]{variant}")), get_holder)
                            }
                            "param" => (
                                String::new(),
                                format!("def find(self, q: {spelled}{variant}) -> str"),
                            ),
                            _ => (String::new(), format!("def find(self) -> {spelled}")),
                        };
                        let api = format!(
                            "{PRELUDE}{leaf_class}{class}{}",
                            port(tag, &[("GET /holder", &signature)])
                        );
                        let built = build_py(&[("ports/api.py", &api)]);
                        let context = format!("{position} {spelled} {variant}");
                        match position {
                            "param" => assert!(
                                built
                                    .contract
                                    .operations
                                    .iter()
                                    .any(|o| o.params.iter().any(|p| p.name == "q")),
                                "{context}: parameter missing; errors {:?}",
                                built.errors
                            ),
                            "return" => assert!(
                                !built.errors.is_empty() || built.contract.operations.len() == 1,
                                "{context}: operation missing"
                            ),
                            _ => assert!(
                                built.contract.schemas.contains_key("Holder"),
                                "{context}: schema missing; errors {:?}",
                                built.errors
                            ),
                        }
                        if variant == " = None"
                            && matches!(position, "dataclass" | "model" | "param")
                        {
                            assert!(built.errors.is_empty(), "{context}: {:?}", built.errors);
                        }
                        let operations: Vec<_> = built
                            .contract
                            .operations
                            .iter()
                            .map(|o| {
                                (
                                    o.params.clone(),
                                    o.body.clone(),
                                    o.success,
                                    o.response.clone(),
                                    o.errors.clone(),
                                )
                            })
                            .collect();
                        let meaning = (
                            built.contract.schemas.clone(),
                            operations,
                            built.errors.is_empty(),
                        );
                        match &first {
                            None => first = Some(meaning),
                            Some(expected) => assert_eq!(&meaning, expected, "{context}"),
                        }
                    }
                }
            }
        }
    }

    /// S30: Any unreadable type on the wire is an error
    #[test]
    fn s30_any_unreadable_type_at_any_depth_is_an_error() {
        let tag = port_tag();
        let wrappers = [
            "{}",
            "list[{}]",
            "Sequence[{}]",
            "tuple[{}, ...]",
            "dict[str, {}]",
            "Mapping[str, {}]",
            "Optional[{}]",
        ];
        for refused in REFUSED_TYPES {
            for outer in wrappers {
                for inner in wrappers {
                    let ty = outer.replace("{}", &inner.replace("{}", refused));
                    let default = if ty.starts_with("Optional[") {
                        " = None"
                    } else {
                        ""
                    };
                    for position in ["field", "deep-field", "param", "return"] {
                        let (class, route, signature, needle) = match position {
                        "field" => (
                            format!("\n@dataclass\nclass Holder:\n    value: {ty}{default}\n"),
                            "GET /holder",
                            "def get_holder(self) -> Holder".to_string(),
                            "    value: ",
                        ),
                        "deep-field" => (
                            format!(
                                "\nclass Inner(TypedDict):\n    value: {}\n\n@dataclass\nclass Holder:\n    inner: list[Inner]\n",
                                if default.is_empty() {
                                    ty.clone()
                                } else {
                                    format!("NotRequired[{ty}]")
                                }
                            ),
                            "GET /holder",
                            "def get_holder(self) -> Holder".to_string(),
                            "    value: ",
                        ),
                        "param" => (
                            String::new(),
                            "POST /holder",
                            format!("def send(self, value: {ty}{default}) -> None"),
                            "def send",
                        ),
                        _ => (
                            String::new(),
                            "GET /holder",
                            format!("def fetch(self) -> {ty}"),
                            "def fetch",
                        ),
                    };
                        let api = format!(
                            "{PRELUDE}{REFUSED_DECLS}{class}{}",
                            port(tag, &[(route, &signature)])
                        );
                        let line = line_of(&api, needle);
                        let built = build_py(&[("ports/api.py", &api)]);
                        let context = format!("{position}: {ty}");
                        assert!(
                            built
                                .errors
                                .iter()
                                .any(|e| e.file == "ports/api.py" && e.line == line),
                            "{context}: expected an error at line {line}, got {:?}",
                            built.errors
                        );
                        assert_no_empty_schema(&built, &context);
                        for schema in built.contract.schemas.values() {
                            if let ApiSchema::Object(fields) = schema {
                                // `left`, `right` and `a` belong to refused classes.
                                for (name, field_ty, _) in fields {
                                    assert!(
                                        !(name == "value" && is_guess(field_ty)),
                                        "{context}: guessed type {field_ty:?} published"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

mod part_4 {
    #[allow(unused_imports)]
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use serde_json::Value;

    // ── helpers ──────────────────────────────────────────────

    struct Project {
        _tmp: tempfile::TempDir,
        root: PathBuf,
    }

    fn project(files: &[(&str, &str)]) -> Project {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("bookmarks");
        fs::create_dir_all(&root).unwrap();
        for (path, text) in files {
            write(&root, path, text);
        }
        Project { _tmp: tmp, root }
    }

    fn write(root: &Path, path: &str, text: &str) {
        let full = root.join(path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(full, text).unwrap();
    }

    struct Out {
        ok: bool,
        stdout: String,
        all: String,
    }

    fn hexa(args: &[&str]) -> Out {
        let out = Command::new(env!("CARGO_BIN_EXE_hexa"))
            .args(args)
            .env("NO_COLOR", "1")
            .output()
            .expect("run hexa");
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        Out {
            ok: out.status.success(),
            all: format!("{stdout}\n{stderr}"),
            stdout,
        }
    }

    fn api(cmd: &str, root: &Path) -> Out {
        hexa(&["api", cmd, root.to_str().unwrap()])
    }

    fn spec_to_stdout(root: &Path) -> Out {
        hexa(&["api", "spec", root.to_str().unwrap(), "--out", "-"])
    }

    fn document(root: &Path) -> Value {
        let out = spec_to_stdout(root);
        assert!(out.ok, "hexa api spec failed:\n{}", out.all);
        let start = out
            .stdout
            .find('{')
            .unwrap_or_else(|| panic!("no JSON document:\n{}", out.all));
        serde_json::from_str(out.stdout[start..].trim())
            .unwrap_or_else(|e| panic!("bad JSON ({e}):\n{}", out.all))
    }

    fn without_identity(mut doc: Value) -> Value {
        if let Some(info) = doc.get_mut("info").and_then(Value::as_object_mut) {
            info.remove("title");
            info.remove("version");
            info.remove("x-generated-by");
        }
        doc
    }

    fn analysis(root: &Path) -> Value {
        let out = hexa(&["analyze", root.to_str().unwrap(), "--json"]);
        let start = out
            .stdout
            .find('{')
            .unwrap_or_else(|| panic!("no analysis JSON:\n{}", out.all));
        let mut stream =
            serde_json::Deserializer::from_str(&out.stdout[start..]).into_iter::<Value>();
        stream
            .next()
            .expect("analysis JSON")
            .unwrap_or_else(|e| panic!("bad analysis JSON ({e}):\n{}", out.all))
    }

    fn find_key<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
        match v {
            Value::Object(m) => m
                .get(key)
                .or_else(|| m.values().find_map(|c| find_key(c, key))),
            Value::Array(a) => a.iter().find_map(|c| find_key(c, key)),
            _ => None,
        }
    }

    fn api_findings(v: &Value) -> &Value {
        find_key(v, "api").unwrap_or_else(|| panic!("analysis has no api findings: {v}"))
    }

    fn score(v: &Value) -> u64 {
        find_key(v, "health_score")
            .or_else(|| find_key(v, "score"))
            .and_then(Value::as_u64)
            .unwrap_or_else(|| panic!("analysis has no score: {v}"))
    }

    fn ungraded(v: &Value) -> u64 {
        find_key(v, "ungraded_python_files")
            .and_then(Value::as_u64)
            .unwrap_or(0)
    }

    const PYPROJECT: &str = "[project]\nname = \"bookmarks\"\nversion = \"0.1.0\"\n";
    const GOMOD: &str = "module example.com/bookmarks\n\ngo 1.22\n";

    const PY_PORT: &str = r#"from dataclasses import dataclass
from typing import Protocol


@dataclass
class Bookmark:
    id: str
    url: str


# Bookmarks.
# @hexa:api
class Bookmarks(Protocol):
    # Fetch one bookmark.
    # @hexa:api GET /bookmarks/{id}
    # @hexa:status 404 not found
    def get_bookmark(self, id: str) -> Bookmark: ...

    # List bookmarks.
    # @hexa:api GET /bookmarks
    def list_bookmarks(self, tag: str) -> list[Bookmark]: ...
"#;

    fn line_of(source: &str, needle: &str) -> u64 {
        source
            .lines()
            .position(|l| l.contains(needle))
            .expect("needle") as u64
            + 1
    }

    fn assert_says_ungraded(out: &Out, root: &Path, count: u64) {
        let text = out.all.replace(root.to_str().unwrap(), "<root>");
        let lower = text.to_lowercase();
        assert!(
            lower.contains("python"),
            "should speak of Python files:\n{text}"
        );
        let wanted = count.to_string();
        assert!(
            text.split(|c: char| !c.is_ascii_alphanumeric())
                .any(|t| t == wanted),
            "should say {count} Python files were left ungraded:\n{text}"
        );
        assert!(lower.contains("grade"), "should speak of grading:\n{text}");
        assert!(
            ["pyproject", "project.json", ".hexa", "setup.py", "language"]
                .iter()
                .any(|h| lower.contains(h)),
            "should say how to have them graded:\n{text}"
        );
    }

    // ── the shared document (S31) ────────────────────────────

    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                if !["target", ".git", "node_modules"].contains(&name.as_str()) {
                    walk(&path, out);
                }
            } else if kind.is_file() {
                out.push(path);
            }
        }
    }

    fn shared_document() -> (PathBuf, Value) {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let workspace = manifest.parent().unwrap_or(manifest);
        let mut files = Vec::new();
        walk(workspace, &mut files);
        files.sort();
        files
            .into_iter()
            .filter(|p| p.file_name().is_some_and(|n| n == "expected.openapi.json"))
            .find_map(|p| {
                let doc: Value = serde_json::from_str(&fs::read_to_string(&p).ok()?).ok()?;
                let is_bookmarks = doc["paths"]
                    .as_object()?
                    .keys()
                    .any(|k| k.contains("bookmark"));
                is_bookmarks.then_some((p, doc))
            })
            .expect("the shared bookmarks expected.openapi.json")
    }

    /// The port's name and tag text, as a sibling fixture writes them.
    fn sibling_port(dir: &Path) -> (String, String) {
        let mut files = Vec::new();
        walk(dir, &mut files);
        files.sort();
        for file in files {
            let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !["ts", "go", "rs"].contains(&ext) {
                continue;
            }
            let Ok(source) = fs::read_to_string(&file) else {
                continue;
            };
            let mut tag: Option<String> = None;
            for line in source.lines() {
                let t = line.trim();
                if t.starts_with('/') || t.starts_with('*') || t.starts_with('#') {
                    if let Some((_, rest)) = t.split_once("@hexa:api") {
                        tag = Some(rest.trim().trim_end_matches("*/").trim().to_string());
                    }
                    continue;
                }
                if let Some(text) = tag.take() {
                    let words: Vec<&str> = t
                        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                        .filter(|w| !w.is_empty())
                        .collect();
                    let name = if let Some(i) = words.iter().position(|w| *w == "trait") {
                        words.get(i + 1)
                    } else if let Some(i) = words.iter().position(|w| *w == "interface") {
                        if words.first() == Some(&"type") {
                            words.get(1)
                        } else {
                            words.get(i + 1)
                        }
                    } else {
                        None
                    };
                    if let Some(name) = name {
                        return (name.to_string(), text);
                    }
                }
            }
        }
        ("Bookmarks".to_string(), String::new())
    }

    fn py_type(schema: &Value) -> String {
        if let Some(r) = schema["$ref"].as_str() {
            return r.rsplit('/').next().unwrap().to_string();
        }
        for key in ["anyOf", "oneOf"] {
            if let Some(choices) = schema[key].as_array() {
                let inner = choices
                    .iter()
                    .find(|c| c["type"] != "null")
                    .expect("a non-null choice");
                return format!("{} | None", py_type(inner));
            }
        }
        if let Some(types) = schema["type"].as_array() {
            let mut one = schema.clone();
            one["type"] = types
                .iter()
                .find(|t| *t != "null")
                .cloned()
                .expect("a non-null type");
            return format!("{} | None", py_type(&one));
        }
        match schema["type"].as_str() {
            Some("string") => "str".into(),
            Some("integer") => "int".into(),
            Some("number") => "float".into(),
            Some("boolean") => "bool".into(),
            Some("array") => format!("list[{}]", py_type(&schema["items"])),
            Some("object") => format!("dict[str, {}]", py_type(&schema["additionalProperties"])),
            _ => panic!("a schema Python cannot write: {schema}"),
        }
    }

    /// (name, annotation, required), required ones in the order the document lists them.
    fn object_fields(schema: &Value) -> Vec<(String, String, bool)> {
        let props = schema["properties"].as_object().expect("object properties");
        let required: Vec<&str> = schema["required"]
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let mut out: Vec<(String, String, bool)> = required
            .iter()
            .map(|n| (n.to_string(), py_type(&props[*n]), true))
            .collect();
        for (name, s) in props {
            if !required.contains(&name.as_str()) {
                out.push((name.clone(), py_type(s), false));
            }
        }
        out
    }

    fn declared(name: &str, ty: &str, required: bool) -> String {
        if required {
            format!("{name}: {ty}")
        } else {
            format!("{name}: {ty} | None = None")
        }
    }

    fn python_from_document(doc: &Value, port: &str, port_tag: &str) -> String {
        let mut out =
            String::from("from dataclasses import dataclass\nfrom typing import Protocol\n\n");
        if let Some(schemas) = doc["components"]["schemas"].as_object() {
            for (name, schema) in schemas {
                if schema["properties"].is_object() {
                    out.push_str(&format!("\n@dataclass\nclass {name}:\n"));
                    for (f, t, req) in object_fields(schema) {
                        out.push_str(&format!("    {}\n", declared(&f, &t, req)));
                    }
                }
            }
        }
        out.push_str(&format!(
            "\n\n# @hexa:api {port_tag}\nclass {port}(Protocol):\n"
        ));
        for (path, item) in doc["paths"].as_object().expect("paths") {
            for verb in ["get", "post", "put", "patch", "delete"] {
                let Some(op) = item.get(verb) else { continue };
                let id = op["operationId"].as_str().expect("operationId");
                let name = hexa_analysis::ports::split_words(id).join("_");
                if let Some(text) = op["description"]
                    .as_str()
                    .or_else(|| op["summary"].as_str())
                {
                    for line in text.lines() {
                        out.push_str(&format!("    # {line}\n"));
                    }
                }
                let responses = op["responses"].as_object().expect("responses");
                let (code, success) = responses
                    .iter()
                    .find(|(c, _)| c.starts_with('2'))
                    .expect("a success response");
                out.push_str(&format!(
                    "    # @hexa:api {} {path} {code}\n",
                    verb.to_uppercase()
                ));
                for (status, response) in responses {
                    if !status.starts_with('2') && status != "500" {
                        let text = response["description"].as_str().unwrap_or("error");
                        out.push_str(&format!("    # @hexa:status {status} {text}\n"));
                    }
                }
                let mut params = vec!["self".to_string(), "*".to_string()];
                if let Some(list) = op["parameters"].as_array() {
                    for p in list {
                        params.push(declared(
                            p["name"].as_str().unwrap(),
                            &py_type(&p["schema"]),
                            p["required"].as_bool().unwrap_or(false),
                        ));
                    }
                }
                let body = &op["requestBody"]["content"]["application/json"]["schema"];
                if !body.is_null() {
                    if body["properties"].is_object() {
                        for (f, t, req) in object_fields(body) {
                            params.push(declared(&f, &t, req));
                        }
                    } else {
                        params.push(format!("body: {}", py_type(body)));
                    }
                }
                if params.len() == 2 {
                    params.pop();
                }
                let result = &success["content"]["application/json"]["schema"];
                let returns = if result.is_null() {
                    "None".to_string()
                } else {
                    py_type(result)
                };
                out.push_str(&format!(
                    "    def {name}({}) -> {returns}: ...\n\n",
                    params.join(", ")
                ));
            }
        }
        out
    }

    /// S31: The Python bookmarks port renders the shared document
    #[test]
    fn s31_python_bookmarks_port_renders_the_shared_document() {
        let (path, expected) = shared_document();
        let dir = path.parent().unwrap();

        let mut files = Vec::new();
        walk(dir, &mut files);
        files.sort();
        let fixture = files.iter().find(|f| {
            f.extension().is_some_and(|e| e == "py")
                && fs::read_to_string(f).is_ok_and(|s| s.contains("@hexa:api"))
        });
        let source = match fixture {
            Some(f) => fs::read_to_string(f).unwrap(),
            None => {
                let (port, tag) = sibling_port(dir);
                python_from_document(&expected, &port, &tag)
            }
        };

        let p = project(&[
            ("pyproject.toml", PYPROJECT),
            ("src/ports/bookmarks.py", &source),
        ]);
        let actual = document(&p.root);

        assert!(actual["openapi"]
            .as_str()
            .is_some_and(|v| v.starts_with("3.1")));
        assert_eq!(
            without_identity(actual),
            without_identity(expected),
            "fixture:\n{source}"
        );
    }

    /// S32: The API commands, CI gate and grade treat Python like other languages
    #[test]
    fn s32_commands_gate_and_grade_treat_python_like_other_languages() {
        let p = project(&[
            ("pyproject.toml", PYPROJECT),
            ("src/ports/bookmarks.py", PY_PORT),
            (
                "src/adapters/primary/health.py",
                "def health() -> str:\n    return \"ok\"\n",
            ),
        ]);

        let list = api("list", &p.root);
        assert!(list.ok, "{}", list.all);
        let lines: Vec<&str> = list
            .stdout
            .lines()
            .filter(|l| l.contains("bookmarks.py:"))
            .collect();
        assert_eq!(lines.len(), 2, "one line per operation:\n{}", list.all);
        assert!(
            lines
                .iter()
                .any(|l| l.contains("GET") && l.contains("/bookmarks/{id}")),
            "{}",
            list.all
        );

        let spec = api("spec", &p.root);
        assert!(spec.ok, "{}", spec.all);
        let committed: Value =
            serde_json::from_str(&fs::read_to_string(p.root.join("openapi.json")).unwrap())
                .unwrap();
        assert!(
            committed["paths"]["/bookmarks/{id}"]["get"].is_object(),
            "{committed}"
        );
        assert!(
            committed["paths"]["/bookmarks"]["get"].is_object(),
            "{committed}"
        );

        let check = api("check", &p.root);
        assert!(check.ok, "{}", check.all);
        let gate = hexa_cli::commands::api::ci_gate(&p.root)
            .unwrap()
            .expect("an API is declared");
        assert!(gate.0, "{:?}", gate.1);

        let result = analysis(&p.root);
        assert_eq!(ungraded(&result), 0, "the project is graded");
        let findings = api_findings(&result);
        assert_eq!(findings["operations"].as_u64(), Some(2), "{findings}");
        assert_eq!(
            findings["errors"].as_array().map(Vec::len),
            Some(0),
            "{findings}"
        );
        let unserved = findings["unserved"].as_array().expect("unserved");
        assert!(
            unserved
                .iter()
                .any(|u| u.as_str().is_some_and(|s| s.contains("Bookmarks"))),
            "{findings}"
        );

        // Drift: both the command and the gate see it.
        let changed = PY_PORT.replace(
            "# @hexa:api GET /bookmarks\n",
            "# @hexa:api GET /all-bookmarks\n",
        );
        write(&p.root, "src/ports/bookmarks.py", &changed);
        let check = api("check", &p.root);
        assert!(!check.ok, "drift must fail check:\n{}", check.all);
        let gate = hexa_cli::commands::api::ci_gate(&p.root)
            .unwrap()
            .expect("an API is declared");
        assert!(!gate.0, "drift must fail the gate: {:?}", gate.1);
    }

    /// S33: A bad Python contract fails the gate and costs grade
    #[test]
    fn s33_bad_python_contract_fails_the_gate_and_costs_grade() {
        let adapter = "from src.ports.bookmarks import Bookmarks\n\n\ndef serve(port: Bookmarks) -> None:\n    port.list_bookmarks(\"x\")\n";
        let make = |port: &str| {
            project(&[
                ("pyproject.toml", PYPROJECT),
                ("src/ports/bookmarks.py", port),
                ("src/adapters/primary/http.py", adapter),
            ])
        };
        let bad_source = PY_PORT.replace("tag: str", "tag: bytes");
        let bad_line = line_of(&bad_source, "tag: bytes");

        let good = make(PY_PORT);
        let bad = make(&bad_source);

        let spec = api("spec", &good.root);
        assert!(spec.ok, "{}", spec.all);
        let committed = fs::read_to_string(good.root.join("openapi.json")).unwrap();
        write(&bad.root, "openapi.json", &committed);

        assert!(api("check", &good.root).ok);
        let check = api("check", &bad.root);
        assert!(!check.ok, "{}", check.all);
        let place = format!("bookmarks.py:{bad_line}");
        assert!(
            check.all.contains(&place),
            "should name {place}:\n{}",
            check.all
        );

        let gate = hexa_cli::commands::api::ci_gate(&bad.root);
        match gate {
            Ok(Some((passed, lines))) => {
                assert!(!passed, "{lines:?}");
                assert!(lines.iter().any(|l| l.contains(&place)), "{lines:?}");
            }
            other => panic!("the gate must run and fail: {other:?}"),
        }

        let good_result = analysis(&good.root);
        let bad_result = analysis(&bad.root);
        let errors = api_findings(&bad_result)["errors"]
            .as_array()
            .expect("errors")
            .clone();
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(
            errors[0]["file"]
                .as_str()
                .is_some_and(|f| f.ends_with("bookmarks.py")),
            "{errors:?}"
        );
        assert_eq!(errors[0]["line"].as_u64(), Some(bad_line), "{errors:?}");
        assert_eq!(
            api_findings(&good_result)["errors"]
                .as_array()
                .map(Vec::len),
            Some(0)
        );
        let cost = hexa_analysis::domain::ArchAnalysisResult::compute_health_score(0, 0, 0, 0, 0)
            - hexa_analysis::domain::ArchAnalysisResult::compute_health_score(0, 0, 0, 0, 1);
        assert_eq!(
            score(&good_result).saturating_sub(score(&bad_result)),
            u64::from(cost),
            "good: {good_result}\nbad: {bad_result}"
        );
    }

    /// S34: Ungraded Python files are not silently treated as having no tags
    #[test]
    fn s34_ungraded_python_files_are_reported() {
        let other = "def helper() -> str:\n    return \"x\"\n";
        let mut files: Vec<(String, &str)> = vec![
            ("go.mod".into(), GOMOD),
            (
                "src/domain/model.go".into(),
                "package domain\n\ntype Thing struct{}\n",
            ),
            ("src/ports/bookmarks.py".into(), PY_PORT),
        ];
        for i in 0..6 {
            files.push((format!("scripts/tool_{}.py", (b'a' + i) as char), other));
        }
        let refs: Vec<(&str, &str)> = files.iter().map(|(p, t)| (p.as_str(), *t)).collect();
        let p = project(&refs);

        let result = analysis(&p.root);
        assert_eq!(ungraded(&result), 7, "{result}");

        let list = api("list", &p.root);
        assert!(!list.stdout.contains("/bookmarks"), "{}", list.all);
        assert_says_ungraded(&list, &p.root, 7);

        let spec = spec_to_stdout(&p.root);
        assert!(!spec.stdout.contains("/bookmarks"), "{}", spec.all);
        assert_says_ungraded(&spec, &p.root, 7);
    }

    // ── equivalence (S35) ────────────────────────────────────

    fn render(manifest: (&str, &str), file: &str, source: &str) -> Value {
        let p = project(&[manifest, (file, source)]);
        without_identity(document(&p.root))
    }

    const EQ_PY: &str = r#"from dataclasses import dataclass
from typing import Protocol


@dataclass
class Item:
    id: str
    title: str
    tags: list[str]
    labels: dict[str, str]
    score: float
    done: bool
    note: str | None = None


@dataclass
class NewItem:
    title: str
    tags: list[str]


# @hexa:api
class Items(Protocol):
    # Fetch one item.
    # @hexa:api GET /items/{id}
    # @hexa:status 404 not found
    def get_item(self, id: str) -> Item: ...

    # List the items.
    # @hexa:api GET /items
    def list_items(self, tag: str | None = None) -> list[Item]: ...

    # Create an item.
    # @hexa:api POST /items 201
    # @hexa:status 409 already there
    def create_item(self, item: NewItem) -> Item: ...

    # Remove an item.
    # @hexa:api DELETE /items/{id}
    def remove_item(self, id: str) -> None: ...
"#;

    const EQ_TS: &str = r#"export interface Item {
  id: string;
  title: string;
  tags: string[];
  labels: Record<string, string>;
  score: number;
  done: boolean;
  note?: string;
}

export interface NewItem {
  title: string;
  tags: string[];
}

/**
 * @hexa:api
 */
export interface Items {
  /**
   * Fetch one item.
   * @hexa:api GET /items/{id}
   * @hexa:status 404 not found
   */
  getItem(id: string): Promise<Item>;

  /**
   * List the items.
   * @hexa:api GET /items
   */
  listItems(tag?: string): Promise<Item[]>;

  /**
   * Create an item.
   * @hexa:api POST /items 201
   * @hexa:status 409 already there
   */
  createItem(item: NewItem): Promise<Item>;

  /**
   * Remove an item.
   * @hexa:api DELETE /items/{id}
   */
  removeItem(id: string): Promise<void>;
}
"#;

    const INT_PY: &str = r#"from dataclasses import dataclass
from typing import Protocol


@dataclass
class Counter:
    name: str
    count: int
    history: list[int]


# @hexa:api
class Counters(Protocol):
    # Read a counter.
    # @hexa:api GET /counters/{id}
    # @hexa:status 404 not found
    def read_counter(self, id: int) -> Counter: ...

    # Reset a counter.
    # @hexa:api DELETE /counters/{id}
    def reset_counter(self, id: int) -> None: ...
"#;

    const INT_GO: &str = "package ports

import \"context\"

type Counter struct {
\tName    string  `json:\"name\"`
\tCount   int     `json:\"count\"`
\tHistory []int   `json:\"history\"`
}

// @hexa:api
type Counters interface {
\t// Read a counter.
\t// @hexa:api GET /counters/{id}
\t// @hexa:status 404 not found
\tReadCounter(ctx context.Context, id int) (Counter, error)

\t// Reset a counter.
\t// @hexa:api DELETE /counters/{id}
\tResetCounter(ctx context.Context, id int) error
}
";

    /// S35: Equivalent ports give the same contract in every language
    #[test]
    fn s35_equivalent_ports_give_the_same_contract() {
        let ts_manifest = (
            "package.json",
            "{\"name\":\"bookmarks\",\"version\":\"0.1.0\"}\n",
        );
        let py_manifest = ("pyproject.toml", PYPROJECT);

        // Without integers, against TypeScript, over several names.
        for noun in ["items", "entries", "records"] {
            let py = EQ_PY.replace("/items", &format!("/{noun}"));
            let ts = EQ_TS.replace("/items", &format!("/{noun}"));
            let from_py = render(py_manifest, "src/ports/items.py", &py);
            let from_ts = render(ts_manifest, "src/ports/items.ts", &ts);
            assert_eq!(from_py, from_ts, "noun {noun}");
            assert!(
                from_py["paths"][format!("/{noun}/{{id}}")]["get"].is_object(),
                "{from_py}"
            );
        }

        // With integers, against Go.
        let from_py = render(py_manifest, "src/ports/counters.py", INT_PY);
        let from_go = render(("go.mod", GOMOD), "src/ports/counters.go", INT_GO);
        assert_eq!(from_py, from_go);
        assert!(
            from_py["paths"]["/counters/{id}"]["delete"].is_object(),
            "{from_py}"
        );
    }

    /// S36: The API commands read exactly the files the grade reads
    #[test]
    fn s36_api_commands_read_exactly_the_files_the_grade_reads() {
        let untagged = "def helper() -> str:\n    return \"x\"\n";
        let broken = PY_PORT.replace("tag: str", "tag: bytes");
        let go_port = "package ports\n\nimport \"context\"\n\n// @hexa:api\ntype Pings interface {\n\t// Ping.\n\t// @hexa:api GET /pings\n\tPing(ctx context.Context) error\n}\n";

        #[derive(Clone, Copy, Debug, PartialEq)]
        enum Tags {
            None,
            Valid,
            Broken,
        }

        for graded in [true, false] {
            for tags in [Tags::None, Tags::Valid, Tags::Broken] {
                for with_go in [false, true] {
                    for extra in [0u8, 3] {
                        let case =
                            format!("graded={graded} tags={tags:?} go={with_go} extra={extra}");
                        let main = match tags {
                            Tags::None => untagged.to_string(),
                            Tags::Valid => PY_PORT.to_string(),
                            Tags::Broken => broken.clone(),
                        };
                        let mut files: Vec<(String, String)> =
                            vec![("src/ports/bookmarks.py".into(), main)];
                        for i in 0..extra {
                            files.push((
                                format!("src/domain/part_{}.py", (b'a' + i) as char),
                                untagged.into(),
                            ));
                        }
                        if graded {
                            files.push(("pyproject.toml".into(), PYPROJECT.into()));
                        } else {
                            files.push(("go.mod".into(), GOMOD.into()));
                            files.push((
                                "src/domain/model.go".into(),
                                "package domain\n\ntype Thing struct{}\n".into(),
                            ));
                        }
                        if with_go {
                            if graded {
                                files.push(("go.mod".into(), GOMOD.into()));
                            }
                            files.push(("src/ports/pings.go".into(), go_port.into()));
                        }
                        let refs: Vec<(&str, &str)> = files
                            .iter()
                            .map(|(p, t)| (p.as_str(), t.as_str()))
                            .collect();
                        let p = project(&refs);

                        let result = analysis(&p.root);
                        let left = ungraded(&result);
                        let expected_left = if graded { 0 } else { u64::from(extra) + 1 };
                        assert_eq!(left, expected_left, "{case}: {result}");

                        let python_ops = if graded && tags == Tags::Valid { 2 } else { 0 };
                        let go_ops = u64::from(with_go);
                        let findings = api_findings(&result);
                        let errors = findings["errors"].as_array().expect("errors");
                        if graded && tags == Tags::Broken {
                            assert!(
                                errors.iter().any(|e| e["file"]
                                    .as_str()
                                    .is_some_and(|f| f.ends_with("bookmarks.py"))),
                                "{case}: {findings}"
                            );
                        } else {
                            assert!(errors.is_empty(), "{case}: {findings}");
                            assert_eq!(
                                findings["operations"].as_u64(),
                                Some(python_ops + go_ops),
                                "{case}: {findings}"
                            );
                        }

                        let list = api("list", &p.root);
                        let python_lines = list
                            .stdout
                            .lines()
                            .filter(|l| l.contains("bookmarks.py:") && l.contains("GET"))
                            .count() as u64;
                        if graded && tags == Tags::Broken {
                            assert!(list.all.contains("bookmarks.py:"), "{case}:\n{}", list.all);
                        } else {
                            assert!(list.ok || python_ops + go_ops == 0, "{case}:\n{}", list.all);
                            assert_eq!(python_lines, python_ops, "{case}:\n{}", list.all);
                            assert_eq!(
                                list.stdout.contains("/pings"),
                                with_go,
                                "{case}:\n{}",
                                list.all
                            );
                        }
                        if !graded {
                            assert!(!list.all.contains("bookmarks.py"), "{case}:\n{}", list.all);
                            if !with_go {
                                assert_says_ungraded(&list, &p.root, left);
                            }
                        }
                    }
                }
            }
        }
    }
}
