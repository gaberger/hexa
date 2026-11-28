#![allow(unused, clippy::all)]
mod part_1 {
    #![allow(clippy::all)]

    use hexa_analysis::domain::Language;
    use hexa_analysis::import_policy::{judge, Origin, Verdict};

    const POLICY_ID: &str = "domain-imports-only-what-it-is-allowed";

    const PYTHON_IO: [&str; 16] = [
        "subprocess",
        "socket",
        "shutil",
        "tempfile",
        "glob",
        "sqlite3",
        "urllib.request",
        "ftplib",
        "smtplib",
        "poplib",
        "imaplib",
        "ssl",
        "signal",
        "multiprocessing",
        "ctypes",
        "importlib",
    ];

    fn template_text() -> String {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../hexa-cli/assets/templates/ADR-rules.toml"
        );
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read template {path}: {e}"))
    }

    fn table_mentions(table: &toml::value::Table, id: &str) -> bool {
        table.values().any(|v| v.as_str() == Some(id))
    }

    fn string_list(table: &toml::value::Table, key: &str) -> Vec<String> {
        table
            .get(key)
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// (allow, deny) of the scaffolded domain policy.
    fn policy() -> (Vec<String>, Vec<String>) {
        let text = template_text();
        let value: toml::Value = toml::from_str(&text).expect("template parses as TOML");
        let policies = value
            .get("import_policy")
            .and_then(|v| v.as_array())
            .expect("template has [[import_policy]]");
        let tables: Vec<&toml::value::Table> =
            policies.iter().filter_map(|p| p.as_table()).collect();
        let table = tables
            .iter()
            .find(|t| table_mentions(t, POLICY_ID))
            .copied()
            .or_else(|| {
                if tables.len() == 1 {
                    Some(tables[0])
                } else {
                    None
                }
            })
            .expect("template has the domain-imports-only-what-it-is-allowed policy");
        (string_list(table, "allow"), string_list(table, "deny"))
    }

    fn judge_py(raw: &str, allow: &[String], deny: &[String]) -> Verdict {
        judge(Language::Python, raw, Origin::StandardLibrary, allow, deny)
    }

    /// The part of a line before a `#` that is outside quotes.
    fn strip_comment(line: &str) -> &str {
        let mut in_str = false;
        for (i, c) in line.char_indices() {
            match c {
                '"' => in_str = !in_str,
                '#' if !in_str => return &line[..i],
                _ => {}
            }
        }
        line
    }

    fn quoted_strings(s: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut cur = String::new();
        let mut in_str = false;
        for c in s.chars() {
            if c == '"' {
                if in_str {
                    out.push(std::mem::take(&mut cur));
                }
                in_str = !in_str;
            } else if in_str {
                cur.push(c);
            }
        }
        out
    }

    /// The raw lines of the policy's `deny = [ ... ]` array.
    fn deny_lines() -> Vec<String> {
        let text = template_text();
        let lines: Vec<&str> = text.lines().collect();
        // Split into table blocks.
        let mut blocks: Vec<Vec<&str>> = Vec::new();
        let mut current: Vec<&str> = Vec::new();
        for l in &lines {
            let t = l.trim_start();
            if t.starts_with('[') && !current.is_empty() {
                // A table header line (not inside an array value: array items rarely start with '[').
                if t.starts_with("[[")
                    || (t.starts_with('[') && t.trim_end().ends_with(']') && !t.contains('"'))
                {
                    blocks.push(std::mem::take(&mut current));
                }
            }
            current.push(l);
        }
        if !current.is_empty() {
            blocks.push(current);
        }
        let block = blocks
            .iter()
            .find(|b| {
                b.first()
                    .map(|h| h.trim_start().starts_with("[[import_policy]]"))
                    .unwrap_or(false)
                    && b.iter().any(|l| l.contains(POLICY_ID))
            })
            .expect("raw template has the domain policy block");
        let start = block
            .iter()
            .position(|l| {
                let t = l.trim_start();
                t.starts_with("deny") && t.contains('=')
            })
            .expect("domain policy has a deny key");
        let mut out = Vec::new();
        let mut depth = 0i32;
        let mut seen_open = false;
        for l in &block[start..] {
            out.push(l.to_string());
            for c in strip_comment(l).chars() {
                if c == '[' {
                    depth += 1;
                    seen_open = true;
                } else if c == ']' {
                    depth -= 1;
                }
            }
            if seen_open && depth <= 0 {
                break;
            }
        }
        out
    }

    /// S01: A Python domain module importing subprocess is denied by the scaffolded policy
    #[test]
    fn s01_subprocess_is_denied_by_its_entry() {
        let (allow, deny) = policy();
        assert_eq!(
            judge_py("subprocess", &allow, &deny),
            Verdict::DeniedByDeny,
            "subprocess must be denied by the scaffolded policy"
        );
        let without: Vec<String> = deny
            .iter()
            .filter(|d| *d != "subprocess")
            .cloned()
            .collect();
        assert_ne!(
            judge_py("subprocess", &allow, &without),
            Verdict::DeniedByDeny,
            "the denial must come from the `subprocess` entry"
        );
    }

    /// S02: Each newly listed Python module appears under a `# Python` heading in the template's deny list
    #[test]
    fn s02_python_modules_listed_under_python_heading() {
        let (_, deny) = policy();
        for m in PYTHON_IO {
            assert!(deny.iter().any(|d| d == m), "deny list lacks `{m}`");
        }
        let lines = deny_lines();
        let heading = lines
            .iter()
            .position(|l| {
                let t = l.trim_start();
                t.starts_with('#') && t.contains("Python")
            })
            .expect("deny list has a `# Python` heading");
        let mut section = Vec::new();
        for l in &lines[heading + 1..] {
            if l.trim_start().starts_with('#') {
                break;
            }
            section.extend(quoted_strings(strip_comment(l)));
        }
        for m in PYTHON_IO {
            assert!(
                section.iter().any(|s| s == m),
                "`{m}` is not under the `# Python` heading; section is {section:?}"
            );
        }
    }

    /// S03: A submodule of a listed module is denied on the `.` boundary
    #[test]
    fn s03_submodules_are_denied() {
        let (allow, deny) = policy();
        for raw in ["multiprocessing.pool", "importlib.util", "urllib.request"] {
            assert_eq!(
                judge_py(raw, &allow, &deny),
                Verdict::DeniedByDeny,
                "`{raw}` must be denied"
            );
        }
    }

    /// S04: `os` and `http` already listed for other languages deny Python's os and http modules
    #[test]
    fn s04_os_and_http_deny_python_modules() {
        let (allow, deny) = policy();
        assert_eq!(judge_py("os.path", &allow, &deny), Verdict::DeniedByDeny);
        assert_eq!(
            judge_py("http.client", &allow, &deny),
            Verdict::DeniedByDeny
        );
        let no_os: Vec<String> = deny.iter().filter(|d| *d != "os").cloned().collect();
        assert_ne!(judge_py("os.path", &allow, &no_os), Verdict::DeniedByDeny);
        let no_http: Vec<String> = deny.iter().filter(|d| *d != "http").cloned().collect();
        assert_ne!(
            judge_py("http.client", &allow, &no_http),
            Verdict::DeniedByDeny
        );
        // And the Python entries are in place too.
        assert!(deny.iter().any(|d| d == "subprocess"));
    }

    /// S05: `os` and `http` are not listed a second time for Python
    #[test]
    fn s05_os_and_http_listed_once() {
        let (_, deny) = policy();
        assert_eq!(deny.iter().filter(|d| *d == "os").count(), 1, "`os` count");
        assert_eq!(
            deny.iter().filter(|d| *d == "http").count(),
            1,
            "`http` count"
        );
        // The Python section exists and does not repeat them.
        assert!(deny.iter().any(|d| d == "subprocess"));
    }

    /// S06: A module that only shares a prefix with a listed name is not denied
    #[test]
    fn s06_prefix_sharing_names_not_denied() {
        let (allow, deny) = policy();
        assert!(deny.iter().any(|d| d == "signal") && deny.iter().any(|d| d == "glob"));
        for raw in ["signals", "globals_helper"] {
            assert_ne!(
                judge_py(raw, &allow, &deny),
                Verdict::DeniedByDeny,
                "`{raw}` must not be denied by `signal`/`glob`"
            );
        }
    }

    /// S07: A listed Python I/O import in a domain file never passes
    #[test]
    fn s07_socket_never_passes() {
        let (allow, deny) = policy();
        let v = judge_py("socket", &allow, &deny);
        assert_ne!(v, Verdict::Permitted);
        assert_ne!(v, Verdict::OutOfScope);
    }

    /// S08: Every listed Python module and every module below it is denied
    #[test]
    fn s08_every_listed_module_and_below_denied() {
        let (allow, deny) = policy();
        let mut modules: Vec<&str> = PYTHON_IO.to_vec();
        modules.push("os");
        modules.push("http");
        let suffixes = ["", "x", "a.b", "client_v2.sub.leaf"];
        for m in &modules {
            for s in suffixes {
                let raw = if s.is_empty() {
                    m.to_string()
                } else {
                    format!("{m}.{s}")
                };
                assert_eq!(
                    judge_py(&raw, &allow, &deny),
                    Verdict::DeniedByDeny,
                    "`{raw}` must be denied"
                );
            }
        }
    }

    /// S09: Pure standard-library modules pass in the domain
    #[test]
    fn s09_pure_modules_pass() {
        let (allow, deny) = policy();
        assert!(deny.iter().any(|d| d == "subprocess"));
        for raw in [
            "pathlib",
            "json",
            "datetime",
            "dataclasses",
            "typing",
            "enum",
            "decimal",
            "re",
            "io",
        ] {
            assert_eq!(
                judge_py(raw, &allow, &deny),
                Verdict::Permitted,
                "`{raw}` must pass"
            );
        }
    }

    /// S10: `urllib.parse` passes although `urllib.request` is denied
    #[test]
    fn s10_urllib_parse_passes() {
        let (allow, deny) = policy();
        assert_eq!(judge_py("urllib.parse", &allow, &deny), Verdict::Permitted);
        assert_eq!(
            judge_py("urllib.request", &allow, &deny),
            Verdict::DeniedByDeny
        );
        assert_eq!(
            judge_py("urllib.request.urlopen", &allow, &deny),
            Verdict::DeniedByDeny
        );
    }
}

mod part_2 {
    #![allow(dead_code)]

    use hexa_analysis::domain::Language;
    use hexa_analysis::import_policy::{classify, judge, Origin, ProjectNames, Verdict};

    const POLICY: &str = "domain-imports-only-what-it-is-allowed";

    const SIXTEEN: [&str; 16] = [
        "subprocess",
        "socket",
        "shutil",
        "tempfile",
        "glob",
        "sqlite3",
        "urllib.request",
        "ftplib",
        "smtplib",
        "poplib",
        "imaplib",
        "ssl",
        "signal",
        "multiprocessing",
        "ctypes",
        "importlib",
    ];

    fn template_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("hexa-cli")
            .join("assets")
            .join("templates")
            .join("ADR-rules.toml")
    }

    fn template_text() -> String {
        std::fs::read_to_string(template_path())
            .expect("read the scaffolded ADR-rules.toml template")
    }

    fn string_list(v: Option<&toml::Value>) -> Vec<String> {
        v.and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn policy_table(text: &str) -> toml::value::Table {
        let value: toml::Value = toml::from_str(text).expect("template parses as TOML");
        let policies = value
            .get("import_policy")
            .and_then(|v| v.as_array())
            .expect("template has [[import_policy]]");
        for p in policies {
            if let Some(t) = p.as_table() {
                if t.values().any(|v| v.as_str() == Some(POLICY)) {
                    return t.clone();
                }
            }
        }
        panic!("template has no import policy named {POLICY}");
    }

    /// (deny, allow) of the scaffolded domain policy.
    fn template_policy() -> (Vec<String>, Vec<String>) {
        let t = policy_table(&template_text());
        (string_list(t.get("deny")), string_list(t.get("allow")))
    }

    /// The raw lines of the domain policy's deny array.
    fn deny_lines(text: &str) -> Vec<String> {
        let lines: Vec<&str> = text.lines().collect();
        let start = lines
            .iter()
            .position(|l| l.contains(POLICY))
            .expect("policy name in template text");
        let mut out = Vec::new();
        let mut inside = false;
        for l in &lines[start..] {
            let t = l.trim();
            if !inside {
                if t.starts_with("deny") && t.contains('=') {
                    inside = true;
                    let after = t.splitn(2, '[').nth(1).unwrap_or("");
                    out.push(after.to_string());
                    if after.contains(']') && !after.trim_start().starts_with('#') {
                        // single-line array (strip trailing part irrelevant)
                        break;
                    }
                }
                continue;
            }
            if t.starts_with(']') {
                break;
            }
            out.push(t.to_string());
        }
        assert!(inside, "deny array found in template text");
        out
    }

    fn quoted(line: &str) -> Vec<String> {
        let code = match line.find('#') {
            Some(i) if !line[..i].contains('"') || line[..i].matches('"').count() % 2 == 0 => {
                &line[..i]
            }
            _ => line,
        };
        let mut out = Vec::new();
        let mut parts = code.split('"');
        parts.next();
        while let Some(s) = parts.next() {
            out.push(s.to_string());
            parts.next();
        }
        out
    }

    /// Entries listed under the `# Python` heading of the deny list.
    fn python_section(text: &str) -> Vec<String> {
        let lines = deny_lines(text);
        let mut out = Vec::new();
        let mut in_python = false;
        let mut seen_heading = false;
        for l in &lines {
            let t = l.trim();
            if t.starts_with('#') {
                let c = t.trim_start_matches('#').trim();
                if c.starts_with("Python") {
                    in_python = true;
                    seen_heading = true;
                    continue;
                }
                let other = ["Rust", "TypeScript", "Node", "Go", "JavaScript"];
                if other.iter().any(|o| c.starts_with(o)) {
                    in_python = false;
                }
                continue;
            }
            if in_python {
                out.extend(quoted(t));
            }
        }
        assert!(seen_heading, "the deny list has a `# Python` heading");
        out
    }

    fn covered(name: &str, deny: &[String]) -> bool {
        deny.iter()
            .any(|d| name == d || name.starts_with(&format!("{d}.")))
    }

    fn assert_python_entries_present(deny: &[String]) {
        for m in SIXTEEN {
            assert!(
                deny.iter().any(|d| d == m),
                "scaffolded deny lists Python's `{m}`"
            );
        }
    }

    /// S11: The template does not list pure modules, nor `urllib` as a whole
    #[test]
    fn s11_template_does_not_list_pure_modules_nor_urllib() {
        let (deny, _) = template_policy();
        assert_python_entries_present(&deny);
        for pure in [
            "pathlib",
            "io",
            "urllib",
            "urllib.parse",
            "json",
            "datetime",
            "dataclasses",
            "typing",
            "enum",
            "decimal",
            "re",
        ] {
            assert!(
                !deny.iter().any(|d| d == pure),
                "deny list must not contain pure module `{pure}`"
            );
        }
    }

    /// S12: A standard-library import no deny entry covers always passes
    #[test]
    fn s12_uncovered_stdlib_import_passes() {
        let (deny, allow) = template_policy();
        assert_python_entries_present(&deny);
        let names = ProjectNames::default();
        let candidates = [
            "pathlib",
            "io",
            "json",
            "datetime",
            "dataclasses",
            "typing",
            "enum",
            "decimal",
            "re",
            "urllib.parse",
            "collections",
            "itertools",
            "functools",
            "math",
            "string",
            "uuid",
            "fractions",
            "abc",
            "operator",
            "copy",
            "textwrap",
            "statistics",
            "heapq",
            "bisect",
        ];
        let mut checked = 0;
        for n in candidates {
            if covered(n, &deny) {
                continue;
            }
            let origin = classify(Language::Python, n, &names);
            if origin != Origin::StandardLibrary {
                continue;
            }
            checked += 1;
            let v = judge(Language::Python, n, origin, &allow, &deny);
            assert_eq!(v, Verdict::Permitted, "uncovered stdlib `{n}` passes");
        }
        assert!(checked > 0, "at least one stdlib module was checked");
    }

    /// S13: Rust, TypeScript and Go entries are unchanged
    #[test]
    fn s13_other_language_entries_unchanged() {
        let text = template_text();
        let (deny, allow) = template_policy();
        let py = python_section(&text);
        for m in SIXTEEN {
            assert!(py.iter().any(|e| e == m), "`{m}` is under `# Python`");
        }
        let outside: Vec<&String> = deny.iter().filter(|d| !py.contains(d)).collect();
        assert!(!outside.is_empty(), "other languages' entries remain");
        for m in SIXTEEN {
            assert!(
                !outside.iter().any(|d| d.as_str() == m),
                "`{m}` appears only in the Python section"
            );
        }
        assert!(
            outside.iter().any(|d| d.as_str() == "os"),
            "`os` still listed"
        );
        assert!(
            outside.iter().any(|d| d.as_str() == "http"),
            "`http` still listed"
        );

        let names = ProjectNames::default();
        let o = classify(Language::Rust, "std::fs", &names);
        assert_eq!(
            judge(Language::Rust, "std::fs", o, &allow, &deny),
            Verdict::DeniedByDeny,
            "Rust std::fs is denied as before"
        );
        let o = classify(Language::Go, "os/exec", &names);
        assert_eq!(
            judge(Language::Go, "os/exec", o, &allow, &deny),
            Verdict::DeniedByDeny,
            "Go os/exec is denied as before"
        );
    }

    /// S14: Third-party Python packages are not denied by default
    #[test]
    fn s14_third_party_packages_not_denied() {
        let (deny, allow) = template_policy();
        assert_python_entries_present(&deny);
        let added: Vec<String> = SIXTEEN.iter().map(|s| s.to_string()).collect();
        let names = ProjectNames::default();
        for pkg in [
            "requests",
            "sqlalchemy",
            "httpx",
            "psycopg2",
            "boto3",
            "redis",
            "aiohttp",
            "pymongo",
            "django",
            "flask",
        ] {
            let origin = classify(Language::Python, pkg, &names);
            assert_ne!(
                judge(Language::Python, pkg, origin, &allow, &added),
                Verdict::DeniedByDeny,
                "no entry added by the ADR denies `{pkg}`"
            );
            assert!(
                !covered(pkg, &deny),
                "the template's deny list names no third-party package (`{pkg}`)"
            );
        }
    }

    /// S16: The change only adds Python standard-library entries
    #[test]
    fn s16_change_only_adds_python_stdlib_entries() {
        let text = template_text();
        let (deny, _) = template_policy();
        let py = python_section(&text);
        for e in &py {
            assert!(
                SIXTEEN.contains(&e.as_str()),
                "Python-section entry `{e}` is one of the sixteen §1 names"
            );
        }
        for m in SIXTEEN {
            assert_eq!(
                deny.iter().filter(|d| d.as_str() == m).count(),
                1,
                "`{m}` listed exactly once"
            );
        }
        for shared in ["os", "http"] {
            assert_eq!(
                deny.iter().filter(|d| d.as_str() == shared).count(),
                1,
                "`{shared}` stays listed once"
            );
        }
        // B ⊆ A: the prior entries are still there and still deny what they did.
        let names = ProjectNames::default();
        let o = classify(Language::Rust, "std::fs", &names);
        assert_eq!(
            judge(Language::Rust, "std::fs", o, &[], &deny),
            Verdict::DeniedByDeny
        );
        let o = classify(Language::Go, "os/exec", &names);
        assert_eq!(
            judge(Language::Go, "os/exec", o, &[], &deny),
            Verdict::DeniedByDeny
        );
    }
}
