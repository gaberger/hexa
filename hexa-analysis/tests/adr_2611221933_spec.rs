#![allow(unused, clippy::all)]
mod part_1 {
    use hexa_analysis::domain::Language;
    use hexa_analysis::import_policy::{judge, Origin, Verdict};

    fn strs(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    /// S02: A Go deny entry does not deny a pure package whose path starts with it
    #[test]
    fn s02_go_deny_net_does_not_deny_net_netip() {
        let deny = strs(&["net"]);
        let allow: Vec<String> = Vec::new();
        let verdict = judge(
            Language::Go,
            "net/netip",
            Origin::StandardLibrary,
            &allow,
            &deny,
        );
        assert_ne!(verdict, Verdict::DeniedByDeny);
        assert_eq!(verdict, Verdict::Permitted);
    }

    /// S03: Go net/url is not denied by a deny entry of net
    #[test]
    fn s03_go_deny_net_does_not_deny_net_url() {
        let deny = strs(&["net"]);
        let allow: Vec<String> = Vec::new();
        let verdict = judge(
            Language::Go,
            "net/url",
            Origin::StandardLibrary,
            &allow,
            &deny,
        );
        assert_ne!(verdict, Verdict::DeniedByDeny);
    }

    /// S04: A Go deny entry does not reach a package below it on a / boundary, even an I/O one
    #[test]
    fn s04_go_deny_net_does_not_deny_net_http() {
        let deny = strs(&["net"]);
        let allow: Vec<String> = Vec::new();
        let verdict = judge(
            Language::Go,
            "net/http",
            Origin::StandardLibrary,
            &allow,
            &deny,
        );
        assert_ne!(verdict, Verdict::DeniedByDeny);
    }

    /// S05: A Go deny entry of os does not deny os/user or os/signal
    #[test]
    fn s05_go_deny_os_does_not_deny_os_user_or_os_signal() {
        let deny = strs(&["os"]);
        let allow: Vec<String> = Vec::new();
        for raw in ["os/user", "os/signal"] {
            let verdict = judge(Language::Go, raw, Origin::StandardLibrary, &allow, &deny);
            assert_ne!(verdict, Verdict::DeniedByDeny, "{raw} was denied by \"os\"");
        }
    }
}

mod part_2 {
    #![allow(clippy::all)]

    use hexa_analysis::domain::Language;
    use hexa_analysis::import_policy::{judge, Origin, Verdict};

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    fn template_text() -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("hexa-cli")
            .join("assets")
            .join("templates")
            .join("ADR-rules.toml");
        std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read template {}: {e}", path.display()))
    }

    fn collect_deny(value: &toml::Value, out: &mut Vec<String>) {
        match value {
            toml::Value::Table(t) => {
                for (k, v) in t {
                    if k == "deny" {
                        if let toml::Value::Array(items) = v {
                            for item in items {
                                if let toml::Value::String(x) = item {
                                    out.push(x.clone());
                                }
                            }
                        }
                    }
                    collect_deny(v, out);
                }
            }
            toml::Value::Array(items) => {
                for item in items {
                    collect_deny(item, out);
                }
            }
            _ => {}
        }
    }

    /// Text fallback: every quoted string inside a `deny = [ ... ]` array.
    fn scan_deny(text: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut rest = text;
        while let Some(i) = rest.find("deny") {
            let after = &rest[i + 4..];
            let trimmed = after.trim_start();
            if let Some(eq) = trimmed.strip_prefix('=') {
                let eq = eq.trim_start();
                if let Some(body) = eq.strip_prefix('[') {
                    if let Some(end) = body.find(']') {
                        let inner = &body[..end];
                        let mut parts = inner.split('"');
                        parts.next();
                        while let Some(q) = parts.next() {
                            out.push(q.to_string());
                            parts.next();
                        }
                    }
                }
            }
            rest = after;
        }
        out
    }

    fn template_deny() -> Vec<String> {
        let text = template_text();
        let mut out = Vec::new();
        match text.parse::<toml::Value>() {
            Ok(v) => collect_deny(&v, &mut out),
            Err(_) => out = scan_deny(&text),
        }
        if out.is_empty() {
            out = scan_deny(&text);
        }
        assert!(!out.is_empty(), "template has no deny entries");
        out
    }

    const EXPECTED_GO_DENY: &[&str] = &[
        "os",
        "os/exec",
        "os/signal",
        "os/user",
        "net",
        "net/http",
        "net/http/httptest",
        "net/http/httputil",
        "net/http/pprof",
        "net/rpc",
        "net/rpc/jsonrpc",
        "net/smtp",
        "database/sql",
        "io/ioutil",
        "plugin",
        "http",
    ];

    
    /// S16: The scaffolded Go deny names each I/O package explicitly
    #[test]
    fn s16_template_go_deny_names_each_io_package() {
        let deny = template_deny();
        for e in EXPECTED_GO_DENY {
            assert!(
                deny.iter().any(|d| d == e),
                "template deny lacks {e}: {deny:?}"
            );
        }
        for d in &deny {
            let go_family = ["net/", "os/", "database/", "io/"]
                .iter()
                .any(|p| d.starts_with(p));
            if go_family {
                assert!(
                    EXPECTED_GO_DENY.contains(&d.as_str()),
                    "template deny has unexpected Go entry {d}"
                );
            }
        }
    }

    /// S18: A scaffolded domain may use pure network-name packages
    #[test]
    fn s18_scaffolded_domain_may_use_pure_packages() {
        let deny = template_deny();
        let allow: Vec<String> = vec![];
        for p in ["net/netip", "net/url", "net/mail", "net/textproto"] {
            assert_eq!(
                judge(Language::Go, p, Origin::StandardLibrary, &allow, &deny),
                Verdict::Permitted,
                "{p} should be permitted as standard library"
            );
        }
    }

    /// S19: The scaffolded Go deny does not list the pure packages
    #[test]
    fn s19_template_does_not_list_pure_packages() {
        let deny = template_deny();
        for p in ["net/netip", "net/url", "net/mail", "net/textproto"] {
            assert!(!deny.iter().any(|d| d == p), "template deny lists {p}");
        }
        // And the template does name the I/O packages explicitly, as the decision requires.
        assert!(
            deny.iter().any(|d| d == "net/smtp"),
            "template deny lacks net/smtp"
        );
    }

    /// S20: The template keeps the bare http entry
    #[test]
    fn s20_template_keeps_bare_http() {
        let deny = template_deny();
        assert!(
            deny.iter().any(|d| d == "http"),
            "template deny dropped http: {deny:?}"
        );
        assert!(
            deny.iter().any(|d| d == "os/signal"),
            "template deny lacks os/signal"
        );
    }
}

mod part_3 {
    #![allow(clippy::all)]

    use hexa_analysis::domain::Language;
    use hexa_analysis::import_policy::{judge, Origin, Verdict};

    fn template_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("hexa-cli")
            .join("assets")
            .join("templates")
            .join("ADR-rules.toml")
    }

    fn collect_deny(value: &toml::Value, out: &mut Vec<String>) {
        match value {
            toml::Value::Table(t) => {
                for (k, v) in t {
                    if k == "deny" {
                        if let toml::Value::Array(items) = v {
                            for item in items {
                                if let toml::Value::String(s) = item {
                                    out.push(s.clone());
                                }
                            }
                        }
                    }
                    collect_deny(v, out);
                }
            }
            toml::Value::Array(items) => {
                for item in items {
                    collect_deny(item, out);
                }
            }
            _ => {}
        }
    }

    /// Fallback for a template that is not standalone TOML: scan every
    /// `deny = [ … ]` array and collect the quoted strings in it.
    fn scan_deny_text(text: &str, out: &mut Vec<String>) {
        let mut rest = text;
        while let Some(pos) = rest.find("deny") {
            let after = &rest[pos + 4..];
            let trimmed = after.trim_start();
            if let Some(eq_rest) = trimmed.strip_prefix('=') {
                let eq_rest = eq_rest.trim_start();
                if let Some(arr) = eq_rest.strip_prefix('[') {
                    if let Some(end) = arr.find(']') {
                        let body = &arr[..end];
                        let mut parts = body.split('"');
                        parts.next();
                        while let Some(s) = parts.next() {
                            out.push(s.to_string());
                            parts.next();
                        }
                    }
                }
            }
            rest = after;
        }
    }

    fn scaffolded_deny() -> Vec<String> {
        let path = template_path();
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "cannot read scaffolded rules template {}: {e}",
                path.display()
            )
        });
        let mut out = Vec::new();
        match text.parse::<toml::Value>() {
            Ok(v) => collect_deny(&v, &mut out),
            Err(_) => scan_deny_text(&text, &mut out),
        }
        if out.is_empty() {
            scan_deny_text(&text, &mut out);
        }
        assert!(
            !out.is_empty(),
            "no deny entries found in the scaffolded rules template {}",
            path.display()
        );
        out.sort();
        out.dedup();
        out
    }
}
