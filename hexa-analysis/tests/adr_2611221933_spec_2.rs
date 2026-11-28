#![allow(unused, clippy::all)]
#![allow(unused, clippy::all)]

use hexa_analysis::domain::Language;
use hexa_analysis::import_policy::{judge, Origin, Verdict};

fn strs(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

/// S14: A Go standard-library deny entry denies exactly the path equal to it; a module deny entry covers the packages below it
#[test]
fn s14_go_standard_library_deny_is_exact_while_module_deny_covers_subpackages() {
    let deny = strs(&["net", "github.com/jackc/pgx/v5"]);
    let allow: Vec<String> = Vec::new();

    // Standard-library entries are exact:
    let verdict_net = judge(Language::Go, "net", Origin::StandardLibrary, &allow, &deny);
    assert_eq!(verdict_net, Verdict::DeniedByDeny);

    let verdict_netip = judge(
        Language::Go,
        "net/netip",
        Origin::StandardLibrary,
        &allow,
        &deny,
    );
    assert_ne!(verdict_netip, Verdict::DeniedByDeny);

    let verdict_net_http = judge(
        Language::Go,
        "net/http",
        Origin::StandardLibrary,
        &allow,
        &deny,
    );
    assert_ne!(verdict_net_http, Verdict::DeniedByDeny);

    // Module entries (with a dot in the first segment) cover packages below them on a / boundary:
    let verdict_module_parent = judge(
        Language::Go,
        "github.com/jackc/pgx/v5",
        Origin::External,
        &allow,
        &deny,
    );
    assert_eq!(verdict_module_parent, Verdict::DeniedByDeny);

    let verdict_module_child = judge(
        Language::Go,
        "github.com/jackc/pgx/v5/pgxpool",
        Origin::External,
        &allow,
        &deny,
    );
    assert_eq!(verdict_module_child, Verdict::DeniedByDeny);
}
