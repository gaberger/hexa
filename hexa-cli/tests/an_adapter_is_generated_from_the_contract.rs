//! The HTTP adapter is generated from the contract (ADR-2610100005).
//!
//! The judge is not the generator: each language's generated adapter is
//! built into a real server, around a port implementation written here from
//! the contract, and `hexa api test` (ADR-2610092329) must prove all four
//! operations. Then the grade must hold: the adapter imports the port only,
//! and the port is no longer unserved.
//!
//! Go runs wherever `go` is on PATH, as the scaffold gate does. TypeScript
//! needs an `npm install` (HEXA_TEST_NPM=1) and Rust needs crates.io for axum
//! (HEXA_TEST_CARGO_NET=1); CI sets both.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/api")
}

fn copy_of(lang: &str) -> tempfile::TempDir {
    fn copy(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for e in std::fs::read_dir(from).unwrap().flatten() {
            let dest = to.join(e.file_name());
            if e.file_type().unwrap().is_dir() {
                copy(&e.path(), &dest);
            } else {
                std::fs::copy(e.path(), dest).unwrap();
            }
        }
    }
    let d = tempfile::tempdir().unwrap();
    copy(&fixtures().join(lang), d.path());
    d
}

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn hexa(cwd: &Path, args: &[&str]) -> Output {
    let home = tempfile::tempdir().unwrap();
    Command::new(env!("CARGO_BIN_EXE_hexa"))
        .args(args)
        .current_dir(cwd)
        .env("HOME", home.path())
        .output()
        .unwrap()
}

fn text(out: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
}

fn run(cwd: &Path, program: &str, args: &[&str], env: &[(&str, &str)]) {
    let out = Command::new(program).args(args).current_dir(cwd).envs(env.iter().copied()).output().unwrap();
    assert!(out.status.success(), "{program} {args:?} failed:\n{}", text(&out));
}

fn have(tool: &str) -> bool {
    Command::new(tool).arg("version").output().is_ok_and(|o| o.status.success())
}

/// A started server, killed when dropped.
struct Server(Child, String);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Start `program`; its first line of output is the base URL it listens on.
fn start(cwd: &Path, program: &str, args: &[&str]) -> Server {
    let mut child = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
    assert!(line.starts_with("http://"), "the server did not say where it listens: {line:?}");
    Server(child, line.trim().to_string())
}

/// The generated adapter, served, proves every operation and grades clean.
fn assert_proven_and_graded(root: &Path, server: &Server) {
    let out = hexa(root, &["api", "test", ".", "--base-url", &server.1]);
    let all = text(&out);
    assert_eq!(out.status.code(), Some(0), "{all}");
    assert!(all.contains("4 proven"), "{all}");

    let out = hexa(root, &["analyze", ".", "--json"]);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
    assert_eq!(v["api"]["unserved"], serde_json::json!([]), "{}", text(&out));
    assert_eq!(v["api"]["errors"], serde_json::json!([]), "{}", text(&out));
    assert_eq!(v["boundary_violations"], serde_json::json!([]), "{}", text(&out));
}

fn generate(root: &Path) -> String {
    let out = hexa(root, &["api", "adapter", "."]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));
    text(&out)
}

#[test]
fn the_go_adapter_serves_the_contract() {
    if !have("go") {
        eprintln!("skipping: no go on PATH");
        return;
    }
    let d = copy_of("go");
    let root = d.path();
    generate(root);
    assert!(root.join("adapters/primary/httpapi/handler.go").is_file());
    write(
        root,
        "cmd/server/main.go",
        r#"package main

import (
	"context"
	"fmt"
	"net"
	"net/http"
	"os"
	"sync"

	"bookmarks/adapters/primary/httpapi"
	"bookmarks/internal/domain"
	"bookmarks/internal/ports"
)

type notFound struct{}

func (notFound) Error() string   { return "not found" }
func (notFound) HTTPStatus() int { return 404 }

type memory struct {
	mu    sync.Mutex
	items []domain.Bookmark
	next  int
}

func (m *memory) Create(_ context.Context, req ports.NewBookmark) (ports.Bookmark, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.next++
	b := domain.Bookmark{ID: domain.BookmarkID(fmt.Sprintf("bm-%d", m.next)), URL: req.URL, Title: req.Title, Tags: req.Tags, SavedAt: "2026-10-10T00:00:00Z"}
	m.items = append(m.items, b)
	return b, nil
}

func (m *memory) Get(_ context.Context, id string) (ports.Bookmark, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	for _, b := range m.items {
		if string(b.ID) == id {
			return b, nil
		}
	}
	return ports.Bookmark{}, notFound{}
}

func (m *memory) ListByTag(_ context.Context, tag string, _ *string) ([]ports.Bookmark, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	out := []ports.Bookmark{}
	for _, b := range m.items {
		for _, t := range b.Tags {
			if t == tag {
				out = append(out, b)
			}
		}
	}
	return out, nil
}

func (m *memory) Delete(_ context.Context, id string) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	kept := m.items[:0]
	for _, b := range m.items {
		if string(b.ID) != id {
			kept = append(kept, b)
		}
	}
	m.items = kept
	return nil
}

func (m *memory) Stats() int { return len(m.items) }

func main() {
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		os.Exit(1)
	}
	fmt.Println("http://" + ln.Addr().String())
	_ = http.Serve(ln, httpapi.NewHandler(&memory{}))
}
"#,
    );
    // The test's `main` is the composition root, and says so, as a project would.
    write(root, ".hexa/project.json", r#"{"analyze": {"layers": {"cmd/server": "composition-root"}}}"#);
    run(root, "go", &["build", "-o", "server", "./cmd/server"], &[("GOTOOLCHAIN", "local"), ("GOFLAGS", "-mod=mod")]);
    let server = start(root, "./server", &[]);
    assert_proven_and_graded(root, &server);
}

#[test]
fn the_typescript_adapter_serves_the_contract() {
    if std::env::var("HEXA_TEST_NPM").is_err() {
        eprintln!("skipping: set HEXA_TEST_NPM=1 to allow the npm install this gate needs");
        return;
    }
    let d = copy_of("ts");
    let root = d.path();
    generate(root);
    assert!(root.join("src/adapters/primary/http-handler.ts").is_file());
    write(
        root,
        "package.json",
        r#"{ "name": "bookmarks", "type": "module", "private": true,
  "devDependencies": { "typescript": "5.6.3", "@types/node": "22.10.2" } }
"#,
    );
    write(
        root,
        "tsconfig.json",
        r#"{ "compilerOptions": { "target": "ES2022", "module": "NodeNext", "moduleResolution": "NodeNext",
  "strict": true, "outDir": "dist", "rootDir": "src", "types": ["node"], "skipLibCheck": true },
  "include": ["src"] }
"#,
    );
    write(
        root,
        "src/main.ts",
        r#"import { createServer } from 'node:http';
import type { AddressInfo } from 'node:net';
import { createHandler } from './adapters/primary/http-handler.js';
import type { Bookmark, BookmarkApi, NewBookmark } from './core/ports/bookmark-api.js';

class NotFound extends Error {
  readonly status = 404;
}

const items: Bookmark[] = [];
let next = 0;

const api: BookmarkApi = {
  async create(req: NewBookmark): Promise<Bookmark> {
    next += 1;
    const b: Bookmark = { id: `bm-${next}`, url: req.url, title: req.title, tags: req.tags, savedAt: '2026-10-10T00:00:00Z' };
    items.push(b);
    return b;
  },
  async get(id: string): Promise<Bookmark> {
    const b = items.find((x) => x.id === id);
    if (!b) throw new NotFound('not found');
    return b;
  },
  async listByTag(tag: string): Promise<Bookmark[]> {
    return items.filter((b) => b.tags.includes(tag));
  },
  async delete(id: string): Promise<void> {
    const i = items.findIndex((b) => b.id === id);
    if (i >= 0) items.splice(i, 1);
  },
  stats(): number {
    return items.length;
  },
};

const server = createServer(createHandler(api));
server.listen(0, '127.0.0.1', () => {
  const a = server.address() as AddressInfo;
  console.log(`http://127.0.0.1:${a.port}`);
});
"#,
    );
    run(root, "npm", &["install", "--silent", "--no-audit", "--no-fund"], &[]);
    run(root, "npx", &["tsc", "-p", "."], &[]);
    let server = start(root, "node", &["dist/main.js"]);
    assert_proven_and_graded(root, &server);
}

#[test]
fn the_rust_adapter_serves_the_contract() {
    if std::env::var("HEXA_TEST_CARGO_NET").is_err() {
        eprintln!("skipping: set HEXA_TEST_CARGO_NET=1 to allow fetching axum from crates.io");
        return;
    }
    let d = copy_of("rust");
    let root = d.path();
    let said = generate(root);
    assert!(said.contains("axum"), "the verb names what a Rust adapter needs: {said}");
    assert!(root.join("src/adapters/primary/http.rs").is_file());
    write(
        root,
        "Cargo.toml",
        r#"[package]
name = "bookmarks"
version = "0.1.0"
edition = "2021"

[workspace]

[dependencies]
axum = "0.8"
serde = { version = "1", features = ["derive"] }
tokio = { version = "1", features = ["rt-multi-thread", "macros", "net"] }
"#,
    );
    write(root, "src/lib.rs", "pub mod adapters;\npub mod domain;\npub mod ports;\n");
    write(root, "src/adapters/mod.rs", "pub mod primary;\n");
    write(root, "src/adapters/primary/mod.rs", "pub mod http;\n");
    write(
        root,
        "src/main.rs",
        r#"use std::sync::{Arc, Mutex};

use bookmarks::domain::bookmark::{Bookmark, BookmarkId, NewBookmark};
use bookmarks::ports::{ApiError, BookmarkApi};

#[derive(Default)]
struct Memory {
    items: Mutex<Vec<Bookmark>>,
}

impl BookmarkApi for Memory {
    fn create(&self, req: NewBookmark) -> Result<Bookmark, ApiError> {
        let mut items = self.items.lock().unwrap();
        let b = Bookmark {
            id: BookmarkId(format!("bm-{}", items.len() + 1)),
            url: req.url,
            title: req.title,
            tags: req.tags,
            saved_at: "2026-10-10T00:00:00Z".into(),
            note: None,
            rank: 0,
        };
        items.push(b.clone());
        Ok(b)
    }

    fn get(&self, id: &str) -> Result<Bookmark, ApiError> {
        let items = self.items.lock().unwrap();
        items.iter().find(|b| b.id.0 == id).cloned().ok_or(ApiError::NotFound)
    }

    fn list_by_tag(&self, tag: &str, _cursor: Option<String>) -> Result<Vec<Bookmark>, ApiError> {
        let items = self.items.lock().unwrap();
        Ok(items.iter().filter(|b| b.tags.iter().any(|t| t == tag)).cloned().collect())
    }

    fn delete(&self, id: &str) -> Result<(), ApiError> {
        self.items.lock().unwrap().retain(|b| b.id.0 != id);
        Ok(())
    }

    fn stats(&self) -> usize {
        self.items.lock().unwrap().len()
    }
}

#[tokio::main]
async fn main() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    println!("http://{}", listener.local_addr().unwrap());
    let app = bookmarks::adapters::primary::http::router(Arc::new(Memory::default()));
    axum::serve(listener, app).await.unwrap();
}
"#,
    );
    // A target directory that outlives the test, so axum compiles once.
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("api-adapter-rust");
    let target = target.to_str().unwrap();
    run(root, "cargo", &["build", "-q", "--bin", "bookmarks"], &[("CARGO_TARGET_DIR", target)]);
    let server = start(root, &format!("{target}/debug/bookmarks"), &[]);
    assert_proven_and_graded(root, &server);
}

#[test]
fn an_existing_adapter_is_not_overwritten_without_force() {
    let d = copy_of("go");
    generate(d.path());
    let out = hexa(d.path(), &["api", "adapter", "."]);
    assert_ne!(out.status.code(), Some(0), "{}", text(&out));
    assert!(text(&out).contains("--force"), "{}", text(&out));
    let out = hexa(d.path(), &["api", "adapter", ".", "--force"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));
}

#[test]
fn out_must_stay_inside_the_project() {
    let d = copy_of("ts");
    let out = hexa(d.path(), &["api", "adapter", ".", "--out", "../escaped.ts"]);
    assert_ne!(out.status.code(), Some(0), "{}", text(&out));
    assert!(!d.path().parent().unwrap().join("escaped.ts").exists());
}

#[test]
fn two_tagged_ports_need_a_choice() {
    let d = copy_of("rust");
    let ports = d.path().join("src/ports/mod.rs");
    let mut src = std::fs::read_to_string(&ports).unwrap();
    src.push_str(
        "\n/// @hexa:api service=health\npub trait HealthApi: Send + Sync {\n    /// @hexa:api GET /health\n    fn health(&self) -> Result<(), ApiError>;\n}\n",
    );
    std::fs::write(&ports, src).unwrap();
    let out = hexa(d.path(), &["api", "adapter", "."]);
    assert_ne!(out.status.code(), Some(0), "{}", text(&out));
    assert!(text(&out).contains("BookmarkApi") && text(&out).contains("HealthApi"), "{}", text(&out));
    let out = hexa(d.path(), &["api", "adapter", ".", "--port", "HealthApi"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));
    let generated = std::fs::read_to_string(d.path().join("src/adapters/primary/http.rs")).unwrap();
    assert!(generated.contains("\"/health\""), "{generated}");
    assert!(!generated.contains("/bookmarks"), "{generated}");
}

#[test]
fn the_same_contract_generates_the_same_bytes() {
    for lang in ["rust", "go", "ts"] {
        let d = copy_of(lang);
        let a = d.path().join("a.out");
        let b = d.path().join("b.out");
        assert!(hexa(d.path(), &["api", "adapter", ".", "--out", "a.out"]).status.success());
        assert!(hexa(d.path(), &["api", "adapter", ".", "--out", "b.out"]).status.success());
        assert_eq!(std::fs::read(a).unwrap(), std::fs::read(b).unwrap(), "{lang}");
    }
}
