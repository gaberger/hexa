#![allow(unused, clippy::all)]
mod part_1 {
    #![allow(dead_code)]

    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use hexa_analysis::analyzer::ArchAnalyzer;
    use hexa_analysis::domain::{
        ArchAnalysisResult, ExportDeclaration, ExportKind, ImportStatement, ItemCounts, Language,
        ModuleReference,
    };
    use hexa_analysis::ports::{AnalysisError, ArchAnalysisPort, AstPort};

    // ── A minimal TypeScript AST adapter for the tests ───────

    struct FakeTsAst {
        root: PathBuf,
        canonical_root: PathBuf,
    }

    impl FakeTsAst {
        fn new(root: &Path) -> Self {
            let canonical_root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
            Self {
                root: root.to_path_buf(),
                canonical_root,
            }
        }

        fn rel(&self, path: &Path) -> String {
            let rel = path
                .strip_prefix(&self.root)
                .or_else(|_| path.strip_prefix(&self.canonical_root))
                .unwrap_or(path);
            rel.to_string_lossy().replace('\\', "/")
        }

        fn resolve(&self, from_rel: &str, raw: &str) -> String {
            if !raw.starts_with('.') {
                return raw.to_string();
            }
            let mut parts: Vec<&str> = from_rel.split('/').collect();
            parts.pop();
            for seg in raw.split('/') {
                match seg {
                    "." | "" => {}
                    ".." => {
                        parts.pop();
                    }
                    s => parts.push(s),
                }
            }
            let base = parts.join("/");
            for suffix in ["", ".tsx", ".ts", "/index.tsx", "/index.ts"] {
                let candidate = format!("{base}{suffix}");
                let full = self.root.join(&candidate);
                if full.is_file() {
                    return candidate;
                }
            }
            base
        }
    }

    fn ident_end(s: &str) -> usize {
        s.char_indices()
            .find(|(_, c)| !(c.is_alphanumeric() || *c == '_' || *c == '$'))
            .map(|(i, _)| i)
            .unwrap_or(s.len())
    }

    fn quoted(s: &str) -> Option<String> {
        let start = s.find(|c| c == '"' || c == '\'')?;
        let q = s[start..].chars().next()?;
        let rest = &s[start + 1..];
        let end = rest.find(q)?;
        Some(rest[..end].to_string())
    }

    impl AstPort for FakeTsAst {
        fn extract_imports(
            &self,
            path: &Path,
            source: &str,
            _lang: Language,
        ) -> Result<Vec<ImportStatement>, AnalysisError> {
            let from_file = self.rel(path);
            let mut out = Vec::new();
            for (i, line) in source.lines().enumerate() {
                let t = line.trim();
                if !t.starts_with("import ") {
                    continue;
                }
                let Some(from_at) = t.rfind(" from ") else {
                    continue;
                };
                let Some(raw) = quoted(&t[from_at..]) else {
                    continue;
                };
                let clause = &t["import ".len()..from_at];
                let mut names = Vec::new();
                if let (Some(a), Some(b)) = (clause.find('{'), clause.find('}')) {
                    for n in clause[a + 1..b].split(',') {
                        let n = n.trim();
                        let n = n.split(" as ").next().unwrap_or("").trim();
                        if !n.is_empty() {
                            names.push(n.to_string());
                        }
                    }
                }
                out.push(ImportStatement {
                    from_file: from_file.clone(),
                    resolved_path: self.resolve(&from_file, &raw),
                    raw_path: raw,
                    names,
                    line: i + 1,
                });
            }
            Ok(out)
        }

        fn count_items(&self, _source: &str, _lang: Language) -> Result<ItemCounts, AnalysisError> {
            Ok(ItemCounts::default())
        }

        fn module_references(
            &self,
            _source: &str,
            _lang: Language,
        ) -> Result<Vec<ModuleReference>, AnalysisError> {
            Ok(Vec::new())
        }

        fn extract_exports(
            &self,
            path: &Path,
            source: &str,
            _lang: Language,
        ) -> Result<Vec<ExportDeclaration>, AnalysisError> {
            let file = self.rel(path);
            let mut out = Vec::new();
            for (i, line) in source.lines().enumerate() {
                let t = line.trim();
                let Some(mut rest) = t.strip_prefix("export ") else {
                    continue;
                };
                if rest.starts_with("default ") {
                    continue;
                }
                if let Some(r) = rest.strip_prefix("async ") {
                    rest = r;
                }
                let kinds = [
                    ("function ", ExportKind::Function),
                    ("const ", ExportKind::Value),
                    ("let ", ExportKind::Value),
                    ("class ", ExportKind::Type),
                    ("interface ", ExportKind::Type),
                    ("type ", ExportKind::Type),
                    ("enum ", ExportKind::Type),
                ];
                for (kw, kind) in kinds {
                    if let Some(r) = rest.strip_prefix(kw) {
                        let r = r.trim_start();
                        let name = &r[..ident_end(r)];
                        if !name.is_empty() {
                            out.push(ExportDeclaration {
                                file: file.clone(),
                                name: name.to_string(),
                                line: i + 1,
                                hexa_public: false,
                                kind,
                            });
                        }
                        break;
                    }
                }
            }
            Ok(out)
        }

        fn extract_references(
            &self,
            _path: &Path,
            source: &str,
            _lang: Language,
        ) -> Result<HashMap<String, usize>, AnalysisError> {
            let mut counts = HashMap::new();
            let mut cur = String::new();
            let mut in_str: Option<char> = None;
            for c in source.chars() {
                if let Some(q) = in_str {
                    if c == q {
                        in_str = None;
                    }
                    continue;
                }
                if c == '"' || c == '\'' || c == '`' {
                    if !cur.is_empty() {
                        *counts.entry(std::mem::take(&mut cur)).or_insert(0) += 1;
                    }
                    in_str = Some(c);
                    continue;
                }
                if c.is_alphanumeric() || c == '_' || c == '$' {
                    cur.push(c);
                } else if !cur.is_empty() {
                    *counts.entry(std::mem::take(&mut cur)).or_insert(0) += 1;
                }
            }
            if !cur.is_empty() {
                *counts.entry(cur).or_insert(0) += 1;
            }
            counts.retain(|k, _| !k.chars().next().unwrap().is_ascii_digit());
            Ok(counts)
        }
    }

    // ── Helpers ──────────────────────────────────────────────

    fn write(root: &Path, rel: &str, content: &str) {
        let full = root.join(rel);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(full, content).unwrap();
    }

    fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "package.json",
            "{\"name\":\"app\",\"version\":\"1.0.0\"}\n",
        );
        for (rel, content) in files {
            write(dir.path(), rel, content);
        }
        dir
    }

    fn analyze(root: &Path) -> ArchAnalysisResult {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let analyzer = ArchAnalyzer::new(Arc::new(FakeTsAst::new(root)));
        rt.block_on(analyzer.analyze(root))
            .expect("analyze succeeds")
    }

    fn is_dead(r: &ArchAnalysisResult, file: &str, name: &str) -> bool {
        r.dead_exports
            .iter()
            .any(|d| d.file == file && d.export_name == name)
    }

    fn dead_sorted(r: &ArchAnalysisResult) -> Vec<(String, String, usize)> {
        let mut v: Vec<_> = r
            .dead_exports
            .iter()
            .map(|d| (d.file.clone(), d.export_name.clone(), d.line))
            .collect();
        v.sort();
        v
    }

    const BUTTON: &str = "export function Button() { return null; }\nexport const buttonVariants = { primary: \"p\" };\n";
    const APP: &str =
        "import { Button } from \"./Button\";\nexport function App() { return Button(); }\n";

    // ── Scenarios ────────────────────────────────────────────

    /// S01: An export used only by a .test.tsx file beside its component is not dead
    #[test]
    fn s01_export_used_only_by_test_tsx_is_not_dead() {
        let dir = project(&[
            ("src/ui/Button.tsx", BUTTON),
            ("src/ui/App.tsx", APP),
            (
                "src/ui/Button.test.tsx",
                "import { buttonVariants } from \"./Button\";\nconst v = buttonVariants;\n",
            ),
        ]);
        let r = analyze(dir.path());
        assert!(
            !is_dead(&r, "src/ui/Button.tsx", "buttonVariants"),
            "buttonVariants is used by Button.test.tsx, yet reported dead: {:?}",
            r.dead_exports
        );
    }

    /// S04: The rule holds at any depth and at the project root
    #[test]
    fn s04_rule_holds_at_any_depth_and_at_root() {
        let dir = project(&[
            ("Root.tsx", "export const rootThing = 1;\n"),
            (
                "Root.test.tsx",
                "import { rootThing } from \"./Root\";\nconst a = rootThing;\n",
            ),
            (
                "src/features/billing/deep/Invoice.tsx",
                "export const invoiceThing = 2;\n",
            ),
            (
                "src/features/billing/deep/Invoice.spec.tsx",
                "import { invoiceThing } from \"./Invoice\";\nconst b = invoiceThing;\n",
            ),
            ("src/ui/Modal.tsx", "export const modalThing = 3;\n"),
            (
                "src/ui/__tests__/Modal.test.tsx",
                "import { modalThing } from \"../Modal\";\nconst c = modalThing;\n",
            ),
        ]);
        let r = analyze(dir.path());
        let tests = [
            "Root.test.tsx",
            "src/features/billing/deep/Invoice.spec.tsx",
            "src/ui/__tests__/Modal.test.tsx",
        ];
        for t in tests {
            assert!(
                !r.coverage.unclassified.iter().any(|f| f == t),
                "{t} is in coverage.unclassified: {:?}",
                r.coverage.unclassified
            );
        }
        assert_eq!(r.coverage.total, 3, "coverage.total counts a test file");
        for (file, name) in [
            ("Root.tsx", "rootThing"),
            ("src/features/billing/deep/Invoice.tsx", "invoiceThing"),
            ("src/ui/Modal.tsx", "modalThing"),
        ] {
            assert!(
                !is_dead(&r, file, name),
                "{name} in {file} is reported dead: {:?}",
                r.dead_exports
            );
        }
    }

    /// S09: A .tsx test is read exactly as a .ts test with the same content
    #[test]
    fn s09_tsx_test_read_as_ts_test() {
        let test_body = "import { buttonVariants } from \"../ui/Button\";\nexport function helper() { return buttonVariants; }\n";
        for (ts_name, tsx_name) in [
            ("src/widgets/Widget.test.ts", "src/widgets/Widget.test.tsx"),
            ("src/widgets/Widget.spec.ts", "src/widgets/Widget.spec.tsx"),
        ] {
            let base = [("src/ui/Button.tsx", BUTTON), ("src/ui/App.tsx", APP)];
            let mut p_files: Vec<(&str, &str)> = base.to_vec();
            p_files.push((ts_name, test_body));
            let mut q_files: Vec<(&str, &str)> = base.to_vec();
            q_files.push((tsx_name, test_body));
            let p = project(&p_files);
            let q = project(&q_files);
            let rp = analyze(p.path());
            let rq = analyze(q.path());
            assert_eq!(
                dead_sorted(&rp),
                dead_sorted(&rq),
                "dead_exports differ for {tsx_name}"
            );
            assert_eq!(
                rp.coverage.total, rq.coverage.total,
                "coverage.total differs for {tsx_name}"
            );
            assert_eq!(
                rp.coverage.classified, rq.coverage.classified,
                "coverage.classified differs for {tsx_name}"
            );
            let mut up = rp.coverage.unclassified.clone();
            up.sort();
            let mut uq = rq.coverage.unclassified.clone();
            uq.sort();
            assert_eq!(up, uq, "coverage.unclassified differs for {tsx_name}");
            assert_eq!(
                rp.file_count, rq.file_count,
                "file_count differs for {tsx_name}"
            );
            assert_eq!(
                rp.health_score, rq.health_score,
                "health_score differs for {tsx_name}"
            );
        }
    }

    /// S10: For every path ending in .test.tsx or .spec.tsx, the file is excluded and its imports are uses
    #[test]
    fn s10_every_test_or_spec_tsx_is_excluded_and_its_imports_are_uses() {
        let cases = [
            ("src/ui", "A.test.tsx"),
            ("", "B.spec.tsx"),
            ("src/x/y/z", "C.test.tsx"),
            ("src/domain", "D.spec.tsx"),
            ("lib/components/forms", "Field.Input.test.tsx"),
        ];
        for (dir_rel, name) in cases {
            let prefix = if dir_rel.is_empty() {
                String::new()
            } else {
                format!("{dir_rel}/")
            };
            let comp = format!("{prefix}Comp.tsx");
            let test = format!("{prefix}{name}");
            let dir = project(&[
            (
                comp.as_str(),
                "export const onlyTestUses = 1;\n",
            ),
            (
                test.as_str(),
                "import { onlyTestUses } from \"./Comp\";\nexport function testHelper() { return onlyTestUses; }\n",
            ),
        ]);
            let r = analyze(dir.path());
            assert!(
                !r.coverage.unclassified.iter().any(|f| *f == test),
                "{test} is in coverage.unclassified: {:?}",
                r.coverage.unclassified
            );
            assert_eq!(r.coverage.total, 1, "{test} counts in coverage.total");
            assert!(
                !r.dead_exports.iter().any(|d| d.file == test),
                "a dead export is reported in {test}: {:?}",
                r.dead_exports
            );
            assert!(
                !is_dead(&r, &comp, "onlyTestUses"),
                "onlyTestUses, imported by {test}, is reported dead: {:?}",
                r.dead_exports
            );
        }
    }
}

mod part_2 {
    #![allow(dead_code, clippy::all)]

    use std::collections::{BTreeSet, HashMap};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use hexa_analysis::analyzer::{source_files_sync, ArchAnalyzer};
    use hexa_analysis::domain::{
        ArchAnalysisResult, ExportDeclaration, ExportKind, ImportStatement, ItemCounts, Language,
        ModuleReference,
    };
    use hexa_analysis::ports::{AnalysisError, ArchAnalysisPort, AstPort};

    // ── A small, predictable AST adapter for the tests ───────────────────

    struct FakeAst {
        root: PathBuf,
        canon_root: PathBuf,
    }

    impl FakeAst {
        fn new(root: &Path) -> Self {
            let canon_root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
            Self {
                root: root.to_path_buf(),
                canon_root,
            }
        }

        fn rel(&self, path: &Path) -> String {
            let s = if let Ok(r) = path.strip_prefix(&self.root) {
                r.to_path_buf()
            } else if let Ok(r) = path.strip_prefix(&self.canon_root) {
                r.to_path_buf()
            } else {
                path.to_path_buf()
            };
            s.to_string_lossy().replace('\\', "/")
        }

        fn exists(&self, rel: &str) -> bool {
            self.root.join(rel).is_file()
        }

        fn resolve_ts(&self, from_rel: &str, raw: &str) -> String {
            let mut parts: Vec<String> = from_rel.split('/').map(|s| s.to_string()).collect();
            parts.pop();
            for seg in raw.split('/') {
                match seg {
                    "." | "" => {}
                    ".." => {
                        parts.pop();
                    }
                    other => parts.push(other.to_string()),
                }
            }
            let mut base = parts.join("/");
            for ext in [".js", ".jsx"] {
                if let Some(stripped) = base.strip_suffix(ext) {
                    base = stripped.to_string();
                }
            }
            let candidates = [
                base.clone(),
                format!("{base}.ts"),
                format!("{base}.tsx"),
                format!("{base}.jsx"),
                format!("{base}.js"),
                format!("{base}/index.ts"),
            ];
            for c in &candidates {
                if self.exists(c) {
                    return c.clone();
                }
            }
            format!("{base}.ts")
        }
    }

    fn identifiers(source: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut cur = String::new();
        for c in source.chars() {
            if c.is_ascii_alphanumeric() || c == '_' {
                cur.push(c);
            } else if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        }
        if !cur.is_empty() {
            out.push(cur);
        }
        out.into_iter()
            .filter(|w| w.chars().next().map_or(false, |c| !c.is_ascii_digit()))
            .collect()
    }

    fn first_ident(s: &str) -> Option<String> {
        let name: String = s
            .trim_start()
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if name.is_empty() {
            None
        } else {
            Some(name)
        }
    }

    impl AstPort for FakeAst {
        fn extract_imports(
            &self,
            path: &Path,
            source: &str,
            lang: Language,
        ) -> Result<Vec<ImportStatement>, AnalysisError> {
            let from = self.rel(path);
            let mut out = Vec::new();
            for (i, line) in source.lines().enumerate() {
                let t = line.trim();
                match lang {
                    Language::TypeScript => {
                        if let Some(rest) = t.strip_prefix("import") {
                            let names: Vec<String> = match (rest.find('{'), rest.find('}')) {
                                (Some(a), Some(b)) if a < b => rest[a + 1..b]
                                    .split(',')
                                    .map(|n| n.trim().to_string())
                                    .filter(|n| !n.is_empty())
                                    .collect(),
                                _ => Vec::new(),
                            };
                            let q = rest.find(|c| c == '"' || c == '\'');
                            if let Some(qa) = q {
                                let quote = rest.as_bytes()[qa] as char;
                                if let Some(len) = rest[qa + 1..].find(quote) {
                                    let raw = rest[qa + 1..qa + 1 + len].to_string();
                                    let resolved = self.resolve_ts(&from, &raw);
                                    out.push(ImportStatement {
                                        from_file: from.clone(),
                                        raw_path: raw,
                                        resolved_path: resolved,
                                        names,
                                        line: i + 1,
                                    });
                                }
                            }
                        }
                    }
                    Language::Rust => {
                        if let Some(rest) = t.strip_prefix("use crate::") {
                            let rest = rest.trim_end_matches(';');
                            let segs: Vec<&str> = rest.split("::").collect();
                            if segs.len() >= 2 {
                                let module = segs[..segs.len() - 1].join("/");
                                let name = segs[segs.len() - 1].to_string();
                                out.push(ImportStatement {
                                    from_file: from.clone(),
                                    raw_path: format!(
                                        "crate::{}",
                                        segs[..segs.len() - 1].join("::")
                                    ),
                                    resolved_path: format!("src/{module}.rs"),
                                    names: vec![name],
                                    line: i + 1,
                                });
                            }
                        }
                    }
                    _ => {}
                }
            }
            Ok(out)
        }

        fn count_items(&self, _source: &str, _lang: Language) -> Result<ItemCounts, AnalysisError> {
            Ok(ItemCounts::default())
        }

        fn module_references(
            &self,
            _source: &str,
            _lang: Language,
        ) -> Result<Vec<ModuleReference>, AnalysisError> {
            Ok(Vec::new())
        }

        fn extract_exports(
            &self,
            path: &Path,
            source: &str,
            lang: Language,
        ) -> Result<Vec<ExportDeclaration>, AnalysisError> {
            let file = self.rel(path);
            let mut out = Vec::new();
            for (i, line) in source.lines().enumerate() {
                let t = line.trim();
                let found: Option<(String, ExportKind)> = match lang {
                    Language::TypeScript => {
                        if let Some(r) = t.strip_prefix("export function ") {
                            first_ident(r).map(|n| (n, ExportKind::Function))
                        } else if let Some(r) = t.strip_prefix("export const ") {
                            first_ident(r).map(|n| (n, ExportKind::Value))
                        } else {
                            None
                        }
                    }
                    Language::Go => t
                        .strip_prefix("func ")
                        .and_then(first_ident)
                        .filter(|n| n.chars().next().map_or(false, |c| c.is_ascii_uppercase()))
                        .map(|n| (n, ExportKind::Function)),
                    Language::Rust => t
                        .strip_prefix("pub fn ")
                        .and_then(first_ident)
                        .map(|n| (n, ExportKind::Function)),
                    _ => None,
                };
                if let Some((name, kind)) = found {
                    out.push(ExportDeclaration {
                        file: file.clone(),
                        name,
                        line: i + 1,
                        hexa_public: false,
                        kind,
                    });
                }
            }
            Ok(out)
        }

        fn extract_references(
            &self,
            _path: &Path,
            source: &str,
            _lang: Language,
        ) -> Result<HashMap<String, usize>, AnalysisError> {
            let mut map = HashMap::new();
            for id in identifiers(source) {
                *map.entry(id).or_insert(0) += 1;
            }
            Ok(map)
        }
    }

    // ── Helpers ──────────────────────────────────────────────────────────

    fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::Builder::new()
            .prefix("hexa-spec-2611221706-")
            .tempdir()
            .expect("tempdir");
        for (rel, content) in files {
            let p = dir.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, content).unwrap();
        }
        dir
    }

    fn analyze(root: &Path) -> ArchAnalysisResult {
        let analyzer = ArchAnalyzer::new(Arc::new(FakeAst::new(root)));
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        rt.block_on(analyzer.analyze(root)).expect("analyze")
    }

    fn is_dead(r: &ArchAnalysisResult, file: &str, name: &str) -> bool {
        r.dead_exports
            .iter()
            .any(|d| d.file == file && d.export_name == name)
    }

    fn dead_set(r: &ArchAnalysisResult) -> BTreeSet<(String, String)> {
        r.dead_exports
            .iter()
            .map(|d| (d.file.clone(), d.export_name.clone()))
            .collect()
    }

    fn graded(root: &Path) -> BTreeSet<String> {
        source_files_sync(root).into_iter().collect()
    }

    fn unclassified(r: &ArchAnalysisResult) -> BTreeSet<String> {
        r.coverage.unclassified.iter().cloned().collect()
    }

    // ── Scenarios ────────────────────────────────────────────────────────

    /// S14: The change does not extend to other JS/TS test suffixes the ADR does not name
    #[test]
    fn s14_test_jsx_is_not_a_test_file() {
        let dir = project(&[(
            "src/ui/Widget.test.jsx",
            "export function helper() {\n  return 1;\n}\n",
        )]);
        let r = analyze(dir.path());
        // The grade reads no .jsx file, before this ADR and after it.
        assert_eq!(
            r.coverage.total, 0,
            "Widget.test.jsx is read as before (not at all): {:?}",
            r.coverage
        );
        assert!(
            !is_dead(&r, "src/ui/Widget.test.jsx", "helper"),
            "it appears in no finding: {:?}",
            r.dead_exports
        );
        assert!(
            !unclassified(&r).contains("src/ui/Widget.test.jsx"),
            "it appears in no finding: {:?}",
            r.coverage
        );
    }

    /// S17: Adding a .test.tsx file changes nothing about the other files
    #[test]
    fn s17_adding_a_tsx_test_changes_nothing_about_the_other_files() {
        let base: Vec<(&str, &str)> = vec![
        (
            "src/ui/Button.tsx",
            "export function Button(): string {\n  return \"b\";\n}\nexport function Icon(): string {\n  return \"i\";\n}\n",
        ),
        (
            "src/core/math.ts",
            "export function sum(a: number, b: number): number {\n  return a + b;\n}\nexport function diff(a: number, b: number): number {\n  return a - b;\n}\n",
        ),
    ];
        let p = project(&base);
        let rp = analyze(p.path());
        let dead_p = dead_set(&rp);

        let variants: Vec<(&str, &str, Vec<(&str, &str)>)> = vec![
        (
            "src/ui/Button.test.tsx",
            "import { Button } from \"./Button\";\nimport { sum } from \"../core/math\";\n\nButton();\nsum(1, 2);\n",
            vec![("src/ui/Button.tsx", "Button"), ("src/core/math.ts", "sum")],
        ),
        (
            "src/deep/nested/Panel.spec.tsx",
            "import { diff } from \"../../core/math\";\n\ndiff(3, 1);\n",
            vec![("src/core/math.ts", "diff")],
        ),
    ];

        for (t_path, t_src, imported) in variants {
            let mut files = base.clone();
            files.push((t_path, t_src));
            let pp = project(&files);
            let rpp = analyze(pp.path());
            assert_eq!(
                unclassified(&rp),
                unclassified(&rpp),
                "adding {t_path} changes coverage.unclassified"
            );
            assert_eq!(
                rp.coverage.total, rpp.coverage.total,
                "adding {t_path} changes coverage.total"
            );
            let mut expected = dead_p.clone();
            for (f, n) in &imported {
                expected.remove(&(f.to_string(), n.to_string()));
            }
            assert_eq!(
                dead_set(&rpp),
                expected,
                "dead exports after adding {t_path} are P's minus what it imports"
            );
        }
    }
}
