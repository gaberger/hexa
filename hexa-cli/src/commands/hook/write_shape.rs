//! Which files a Bash command would write (ADR-2611160730 §4).
//!
//! An audit found 33 of 35 source edits in one session went through Bash
//! rather than Edit/Write: `python3` heredoc patch scripts, `python3 /tmp/x.py`,
//! `sed -i`, `cat > file`, `cat >> file`, `tee`. The `pre-bash` hook saw every
//! one and let it through. This module reads a command the way a shell would,
//! closely enough to name the files it writes, so the hook can hold those
//! writes to the same gate as an Edit.
//!
//! It is a classifier, not a shell. It returns *candidate* paths, resolved
//! lexically against the command's working directory (following `cd` within
//! the command). Deciding which of them are project source (tracked, not
//! build output or scratch) is [`super::source_gate`]'s job. What it reads:
//!
//! - output redirections `>`, `>>`, `>|`, `&>`, `&>>`, `N>` on any command,
//!   except into `/dev/*` or another descriptor;
//! - `tee [-a] FILE…`;
//! - `sed -i` / `sed -i ''` / `sed -i.bak` / `--in-place` and `perl -pi`:
//!   the file operands;
//! - `cp` / `mv` / `install` destination (and `mv` sources, which it removes);
//!   `dd of=FILE`;
//! - `python*` / `node` / `bun` / `deno` / `ruby` given code by `-c`/`-e`, a
//!   heredoc, a here-string, or a script file (read, bounded): the paths the
//!   code writes when it can see them (`open('p','w')`, `writeFileSync('p'`),
//!   else every path-like string literal and argument once the code contains
//!   a write call it cannot attribute (`open(p, 'w')`, `.write(`);
//! - `bash`/`sh`/`zsh` given `-c STR` or a heredoc: the inner command.
//!
//! A command run by `hexa` is never a candidate: hexa's own verbs gate their
//! writes themselves.

use std::path::{Component, Path, PathBuf};

/// The largest script file read to look for writes.
pub const SCRIPT_READ_CAP: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Word(String),
    Op(&'static str),
    /// Index into the heredoc bodies, standing where the delimiter was.
    HereDoc(usize),
}

const OPS: &[&str] = &[
    "&>>", "<<<", "<<-", "&&", "||", "|&", ";;", "&>", ">>", ">|", ">&", "<<", "<&", ";", "|", "&", ">", "<", "(", ")",
];

/// Split a command line into words and operators, with heredoc bodies cut
/// out and kept by index.
fn tokenize(src: &str) -> (Vec<Tok>, Vec<String>) {
    let chars: Vec<char> = src.chars().collect();
    let mut toks = Vec::new();
    let mut bodies: Vec<String> = Vec::new();
    // Heredocs opened on the current line, waiting for its newline:
    // (body index, delimiter, strip leading tabs).
    let mut pending: Vec<(usize, String, bool)> = Vec::new();
    let mut want_delim: Option<bool> = None;
    let mut word = String::new();
    let mut in_word = false;
    let mut i = 0;

    let flush = |word: &mut String,
                 in_word: &mut bool,
                 toks: &mut Vec<Tok>,
                 want_delim: &mut Option<bool>,
                 pending: &mut Vec<(usize, String, bool)>,
                 bodies: &mut Vec<String>| {
        if !*in_word {
            return;
        }
        let w = std::mem::take(word);
        *in_word = false;
        if let Some(dash) = want_delim.take() {
            let idx = bodies.len();
            bodies.push(String::new());
            pending.push((idx, w, dash));
            toks.push(Tok::HereDoc(idx));
        } else {
            toks.push(Tok::Word(w));
        }
    };

    while i < chars.len() {
        let c = chars[i];
        match c {
            '\n' => {
                flush(&mut word, &mut in_word, &mut toks, &mut want_delim, &mut pending, &mut bodies);
                toks.push(Tok::Op(";"));
                i += 1;
                // Read the bodies of the heredocs opened on this line.
                for (idx, delim, dash) in std::mem::take(&mut pending) {
                    let mut body = String::new();
                    loop {
                        if i >= chars.len() {
                            break;
                        }
                        let end = chars[i..].iter().position(|&c| c == '\n').map(|p| i + p).unwrap_or(chars.len());
                        let line: String = chars[i..end].iter().collect();
                        i = end + 1;
                        let probe = if dash { line.trim_start_matches('\t') } else { line.as_str() };
                        if probe.trim_end() == delim {
                            break;
                        }
                        body.push_str(&line);
                        body.push('\n');
                    }
                    bodies[idx] = body;
                }
            }
            ' ' | '\t' | '\r' => {
                flush(&mut word, &mut in_word, &mut toks, &mut want_delim, &mut pending, &mut bodies);
                i += 1;
            }
            '#' if !in_word => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '\\' => {
                if i + 1 < chars.len() {
                    if chars[i + 1] != '\n' {
                        word.push(chars[i + 1]);
                        in_word = true;
                    }
                    i += 2;
                } else {
                    i += 1;
                }
            }
            '\'' => {
                in_word = true;
                i += 1;
                while i < chars.len() && chars[i] != '\'' {
                    word.push(chars[i]);
                    i += 1;
                }
                i += 1;
            }
            '"' => {
                in_word = true;
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    if chars[i] == '\\' && i + 1 < chars.len() && matches!(chars[i + 1], '"' | '\\' | '$' | '`') {
                        word.push(chars[i + 1]);
                        i += 2;
                        continue;
                    }
                    word.push(chars[i]);
                    i += 1;
                }
                i += 1;
            }
            '$' if chars.get(i + 1) == Some(&'(') || chars.get(i + 1) == Some(&'{') => {
                // A substitution stays inside the word it is part of.
                let (open, close) = if chars[i + 1] == '(' { ('(', ')') } else { ('{', '}') };
                in_word = true;
                word.push('$');
                word.push(open);
                i += 2;
                let mut depth = 1;
                while i < chars.len() && depth > 0 {
                    if chars[i] == open {
                        depth += 1;
                    } else if chars[i] == close {
                        depth -= 1;
                    }
                    word.push(chars[i]);
                    i += 1;
                }
            }
            '`' => {
                in_word = true;
                word.push('`');
                i += 1;
                while i < chars.len() && chars[i] != '`' {
                    word.push(chars[i]);
                    i += 1;
                }
                word.push('`');
                i += 1;
            }
            _ => {
                let rest: String = chars[i..chars.len().min(i + 3)].iter().collect();
                if let Some(op) = OPS.iter().find(|op| rest.starts_with(**op)) {
                    // `2>` and `2>>`: a descriptor number before a redirection
                    // belongs to the operator, not to the words.
                    if in_word && (op.starts_with('>') || op.starts_with('<')) && word.chars().all(|c| c.is_ascii_digit()) {
                        word.clear();
                        in_word = false;
                    }
                    flush(&mut word, &mut in_word, &mut toks, &mut want_delim, &mut pending, &mut bodies);
                    if *op == "<<" || *op == "<<-" {
                        want_delim = Some(*op == "<<-");
                    }
                    toks.push(Tok::Op(op));
                    i += op.len();
                } else {
                    word.push(c);
                    in_word = true;
                    i += 1;
                }
            }
        }
    }
    flush(&mut word, &mut in_word, &mut toks, &mut want_delim, &mut pending, &mut bodies);
    (toks, bodies)
}

/// One simple command: its words, the files its output is redirected into,
/// and any text given to it on stdin by a heredoc or here-string.
#[derive(Debug, Default)]
struct Simple {
    words: Vec<String>,
    out_redirs: Vec<String>,
    stdin_text: Vec<String>,
}

fn simple_commands(src: &str) -> Vec<Simple> {
    let (toks, bodies) = tokenize(src);
    let mut out = Vec::new();
    let mut cur = Simple::default();
    let mut it = toks.into_iter().peekable();
    while let Some(t) = it.next() {
        match t {
            Tok::Word(w) => cur.words.push(w),
            Tok::HereDoc(idx) => cur.stdin_text.push(bodies[idx].clone()),
            Tok::Op(op) => match op {
                ">" | ">>" | ">|" | "&>" | "&>>" | ">&" => {
                    if let Some(Tok::Word(target)) = it.peek().cloned() {
                        it.next();
                        let dup = op == ">&" && (target == "-" || target.chars().all(|c| c.is_ascii_digit()));
                        if !dup {
                            cur.out_redirs.push(target);
                        }
                    }
                }
                "<" | "<&" => {
                    if matches!(it.peek(), Some(Tok::Word(_))) {
                        it.next();
                    }
                }
                "<<<" => {
                    if let Some(Tok::Word(text)) = it.peek().cloned() {
                        it.next();
                        cur.stdin_text.push(text);
                    }
                }
                "<<" | "<<-" => {
                    if let Some(Tok::HereDoc(idx)) = it.peek().cloned() {
                        it.next();
                        cur.stdin_text.push(bodies[idx].clone());
                    }
                }
                _ => {
                    if !cur.words.is_empty() || !cur.out_redirs.is_empty() || !cur.stdin_text.is_empty() {
                        out.push(std::mem::take(&mut cur));
                    }
                }
            },
        }
    }
    if !cur.words.is_empty() || !cur.out_redirs.is_empty() || !cur.stdin_text.is_empty() {
        out.push(cur);
    }
    out
}

/// `a/./b/../c` → `a/c`, without touching the filesystem: the file a command
/// writes may not exist yet.
pub fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn resolve(cwd: &Path, p: &str) -> Option<PathBuf> {
    if p.is_empty() || p.contains('$') || p.contains('`') || p.contains('*') {
        return None;
    }
    let p = if let Some(rest) = p.strip_prefix("~/") {
        dirs::home_dir()?.join(rest)
    } else {
        PathBuf::from(p)
    };
    Some(normalize(&if p.is_absolute() { p } else { cwd.join(p) }))
}

fn is_assignment(w: &str) -> bool {
    let Some((k, _)) = w.split_once('=') else { return false };
    !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !k.starts_with(|c: char| c.is_ascii_digit())
}

/// The words of the command proper: leading `VAR=x` assignments and wrappers
/// like `sudo`, `env`, `time`, `nohup`, `xargs` removed.
fn strip_wrappers(words: &[String]) -> &[String] {
    let mut i = 0;
    while i < words.len() {
        let w = words[i].as_str();
        if is_assignment(w) {
            i += 1;
            continue;
        }
        let base = w.rsplit('/').next().unwrap_or(w);
        match base {
            "sudo" | "env" | "time" | "nohup" | "command" | "exec" | "nice" | "xargs" | "stdbuf" => {
                i += 1;
                while i < words.len() && words[i].starts_with('-') {
                    i += 1;
                }
            }
            "timeout" => {
                i += 1;
                while i < words.len() && words[i].starts_with('-') {
                    i += 1;
                }
                i += 1; // the duration
            }
            _ => break,
        }
    }
    &words[i.min(words.len())..]
}

fn program(w: &str) -> &str {
    w.rsplit('/').next().unwrap_or(w)
}

fn is_python(p: &str) -> bool {
    p.starts_with("python") || p.starts_with("pypy")
}

/// Each simple command in a command line, as the directory it runs in
/// (following `cd`/`pushd` within the line, as [`write_targets`] does) and
/// its words with leading assignments and wrappers like `env` removed.
/// Heredoc bodies and redirections are left out. For classifiers of other
/// command shapes, such as `git commit` (ADR-2611160910 finding 1).
pub fn command_words(command: &str, cwd: &Path) -> Vec<(PathBuf, Vec<String>)> {
    let mut cwd = cwd.to_path_buf();
    let mut out = Vec::new();
    for simple in simple_commands(command) {
        let words = strip_wrappers(&simple.words).to_vec();
        let prog = words.first().map(|w| program(w).to_string()).unwrap_or_default();
        if prog == "cd" || prog == "pushd" {
            if let Some(d) = words[1..].iter().find(|a| !a.starts_with('-')) {
                if let Some(p) = resolve(&cwd, d) {
                    cwd = p;
                }
            }
        }
        if !words.is_empty() {
            out.push((cwd.clone(), words));
        }
    }
    out
}

/// A path word resolved against `cwd`, lexically; `None` for a word with
/// `$`, a backquote or a glob, which only a shell can expand.
pub fn resolve_word(cwd: &Path, word: &str) -> Option<PathBuf> {
    resolve(cwd, word)
}

/// The paths a whole command line would write, as candidates.
/// `read_script` reads a script file named on the command line (at most
/// [`SCRIPT_READ_CAP`] bytes); tests pass a fake.
pub fn write_targets(command: &str, cwd: &Path, read_script: &dyn Fn(&Path) -> Option<String>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect(command, cwd, read_script, &mut out, 0);
    let mut seen = std::collections::HashSet::new();
    out.retain(|p| seen.insert(p.clone()));
    out
}

fn collect(command: &str, cwd: &Path, read_script: &dyn Fn(&Path) -> Option<String>, out: &mut Vec<PathBuf>, depth: u8) {
    if depth > 3 {
        return;
    }
    let mut cwd = cwd.to_path_buf();
    for simple in simple_commands(command) {
        let words = strip_wrappers(&simple.words);
        let prog = words.first().map(|w| program(w)).unwrap_or("");
        if prog == "hexa" {
            continue;
        }
        for r in &simple.out_redirs {
            if r.starts_with("/dev/") {
                continue;
            }
            out.extend(resolve(&cwd, r));
        }
        let args = if words.is_empty() { &[][..] } else { &words[1..] };
        match prog {
            "cd" | "pushd" => {
                if let Some(d) = args.iter().find(|a| !a.starts_with('-')) {
                    if let Some(p) = resolve(&cwd, d) {
                        cwd = p;
                    }
                }
            }
            "tee" => out.extend(args.iter().filter(|a| !a.starts_with('-')).filter_map(|a| resolve(&cwd, a))),
            "sed" | "gsed" => out.extend(sed_targets(args).iter().filter_map(|a| resolve(&cwd, a))),
            "perl" => out.extend(perl_targets(args).iter().filter_map(|a| resolve(&cwd, a))),
            "cp" | "mv" | "install" | "rsync" => {
                out.extend(copy_targets(args, prog == "mv").iter().filter_map(|a| resolve(&cwd, a)))
            }
            "dd" => out.extend(args.iter().filter_map(|a| a.strip_prefix("of=")).filter_map(|a| resolve(&cwd, a))),
            "bash" | "sh" | "zsh" => {
                if let Some(i) = args.iter().position(|a| a == "-c" || (a.starts_with('-') && !a.starts_with("--") && a.ends_with('c'))) {
                    if let Some(inner) = args.get(i + 1) {
                        collect(inner, &cwd, read_script, out, depth + 1);
                    }
                } else if args.iter().all(|a| a.starts_with('-')) {
                    for text in &simple.stdin_text {
                        collect(text, &cwd, read_script, out, depth + 1);
                    }
                }
            }
            p if is_python(p) || matches!(p, "node" | "nodejs" | "bun" | "deno" | "ruby") => {
                if let Some((code, argv, script)) = interpreter_code(p, args, &simple.stdin_text, &cwd, read_script) {
                    // A script that reads its arguments and was handed an absolute
                    // directory joins its own relative names onto it
                    // (`root = Path(sys.argv[1])`): those names are that
                    // directory's, not the place the command runs.
                    let root_arg = code.contains("argv").then(|| argv.iter().find(|a| a.starts_with('/'))).flatten();
                    for lit in code_write_targets(&code, &argv) {
                        // Running a script is not editing it: its own name, in a
                        // usage line, is a literal it merely mentions.
                        let target = match root_arg {
                            Some(root) if !lit.starts_with('/') && !argv.contains(&lit) => resolve(Path::new(root), &lit),
                            _ => resolve(&cwd, &lit),
                        };
                        if target.is_some() && target == script {
                            continue;
                        }
                        out.extend(target);
                    }
                }
            }
            _ => {}
        }
    }
}

/// `sed -i …`: the file operands, or nothing when `-i` is absent.
fn sed_targets(args: &[String]) -> Vec<String> {
    let mut in_place = false;
    let mut script_given = false;
    let mut operands = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if a == "--" {
            operands.extend(args[i + 1..].iter().cloned());
            break;
        } else if a == "-e" || a == "--expression" || a == "-f" || a == "--file" {
            script_given = true;
            i += 2;
            continue;
        } else if a.starts_with("--expression=") || a.starts_with("--file=") {
            script_given = true;
        } else if a == "--in-place" || a.starts_with("--in-place=") {
            in_place = true;
        } else if a.starts_with('-') && a.len() > 1 && !a.starts_with("--") {
            let flags = &a[1..];
            // After `i`, GNU reads the rest of the word as the backup suffix.
            let letters = flags.find('i').map(|p| &flags[..p]).unwrap_or(flags);
            if let Some(pos) = flags.find('i') {
                in_place = true;
                // BSD `-i ''` / `-i .bak`: the suffix is the next word.
                if pos == flags.len() - 1 && flags.len() == 1 {
                    if let Some(next) = args.get(i + 1) {
                        if next.is_empty() || (next.starts_with('.') && next.len() < 10 && !next.contains('/')) {
                            i += 1;
                        }
                    }
                }
            }
            if letters.contains('e') || letters.contains('f') {
                script_given = true;
                i += 2;
                continue;
            }
        } else {
            operands.push(a.to_string());
        }
        i += 1;
    }
    if !in_place {
        return Vec::new();
    }
    if !script_given && !operands.is_empty() {
        operands.remove(0);
    }
    operands
}

/// `perl -pi -e '…' FILE…`: the file operands, or nothing without `-i`.
fn perl_targets(args: &[String]) -> Vec<String> {
    let mut in_place = false;
    let mut code_given = false;
    let mut operands = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if a == "--" {
            operands.extend(args[i + 1..].iter().cloned());
            break;
        }
        if a.starts_with('-') && a.len() > 1 && !a.starts_with("--") {
            let flags = &a[1..];
            if flags.starts_with('i') || flags.contains('i') && !flags.starts_with('e') && !flags.starts_with('E') {
                in_place = true;
            }
            if flags.ends_with('e') || flags.ends_with('E') {
                code_given = true;
                i += 2;
                continue;
            }
        } else {
            operands.push(a.to_string());
        }
        i += 1;
    }
    if !in_place {
        return Vec::new();
    }
    if !code_given && !operands.is_empty() {
        operands.remove(0);
    }
    operands
}

/// `cp`/`mv`/`install`: the destination (`-t DIR` or the last operand); for
/// `mv`, the sources too, which it deletes.
fn copy_targets(args: &[String], is_mv: bool) -> Vec<String> {
    let mut operands = Vec::new();
    let mut target_dir = None;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if a == "-t" || a == "--target-directory" {
            target_dir = args.get(i + 1).cloned();
            i += 2;
            continue;
        }
        if let Some(d) = a.strip_prefix("--target-directory=") {
            target_dir = Some(d.to_string());
        } else if a == "-m" || a == "-o" || a == "-g" || a == "-S" {
            i += 2; // install's option arguments
            continue;
        } else if !a.starts_with('-') {
            operands.push(a.to_string());
        }
        i += 1;
    }
    let mut out = Vec::new();
    let sources: Vec<String> = match target_dir {
        Some(d) => {
            out.push(d);
            operands
        }
        None => {
            if operands.len() < 2 {
                return Vec::new();
            }
            out.push(operands.pop().unwrap());
            operands
        }
    };
    if is_mv {
        out.extend(sources);
    }
    out
}

/// The code an interpreter would run, and the arguments it is given.
fn interpreter_code(
    prog: &str,
    args: &[String],
    stdin_text: &[String],
    cwd: &Path,
    read_script: &dyn Fn(&Path) -> Option<String>,
) -> Option<(String, Vec<String>, Option<PathBuf>)> {
    let python = is_python(prog);
    let code_flags: &[&str] = if python { &["-c"] } else { &["-e", "--eval", "-p", "--print"] };
    // Flags that take a value and are not code.
    let valued: &[&str] = if python { &["-W", "-X", "--check-hash-based-pycs"] } else { &["-r", "--require", "--import", "-I"] };
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if code_flags.contains(&a) {
            return args.get(i + 1).map(|c| (c.clone(), args[i + 2..].to_vec(), None));
        }
        if python && a == "-m" {
            return None; // a module: `python -m pytest`
        }
        if !python && (a == "--test" || a == "test" || a == "run") {
            return None; // a test runner or package script
        }
        if a == "-" {
            return Some((stdin_text.join("\n"), args[i + 1..].to_vec(), None));
        }
        if valued.contains(&a) {
            i += 2;
            continue;
        }
        if a.starts_with('-') {
            i += 1;
            continue;
        }
        let script = resolve(cwd, a)?;
        let mut code = read_script(&script)?;
        code.truncate(floor_char_boundary(&code, SCRIPT_READ_CAP));
        return Some((code, args[i + 1..].to_vec(), Some(script)));
    }
    if stdin_text.is_empty() {
        None
    } else {
        Some((stdin_text.join("\n"), Vec::new(), None))
    }
}

fn floor_char_boundary(s: &str, at: usize) -> usize {
    let mut at = at.min(s.len());
    while !s.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// The paths a piece of Python/JS/Ruby code writes, as written in it.
///
/// Precise first: `open('p', 'w')`, `Path('p').write_text(`,
/// `writeFileSync('p'`, `File.write('p'`, `shutil.copy(a, 'p')`. When the code
/// also writes somewhere it does not spell out (`open(p, 'w')`, a bare
/// `.write(` on a handle, `writeFileSync(p`), every path-like string literal
/// in it and every argument it was given is a candidate: the patch scripts the
/// audit found name their target once, in a variable.
pub fn code_write_targets(code: &str, argv: &[String]) -> Vec<String> {
    use regex::Regex;
    use std::sync::OnceLock;
    static RES: OnceLock<Vec<Regex>> = OnceLock::new();
    static ANY_WRITE: OnceLock<Regex> = OnceLock::new();
    static OPEN_WRITE: OnceLock<Regex> = OnceLock::new();
    static BARE_WRITE: OnceLock<Regex> = OnceLock::new();
    static LIT: OnceLock<Regex> = OnceLock::new();
    let lit = r#"(?:'([^'\n]+)'|"([^"\n]+)"|`([^`\n$]+)`)"#;
    let precise = RES.get_or_init(|| {
        [
            format!(r#"\bopen\(\s*{lit}\s*,\s*(?:mode\s*=\s*)?['"][rbt]*[wax+]"#),
            format!(r#"Path\(\s*{lit}\s*\)\s*\.(?:write_text|write_bytes|open\(\s*['"][wax])"#),
            format!(r#"\b(?:writeFileSync|writeFile|appendFileSync|appendFile|createWriteStream|outputFileSync)\(\s*{lit}"#),
            format!(r#"\b(?:File|IO)\.(?:write|binwrite)\(\s*{lit}"#),
            format!(r#"\bFile\.open\(\s*{lit}\s*,\s*['"][rb]*[wa+]"#),
            format!(r#"\b(?:shutil\.(?:copy\w*|move)|os\.(?:replace|rename)|fs\.(?:copyFileSync|renameSync))\([^,\n]+,\s*{lit}"#),
        ]
        .iter()
        .map(|p| Regex::new(p).expect("write regex"))
        .collect()
    });
    let any_write = ANY_WRITE.get_or_init(|| {
        Regex::new(r#"\bopen\([^\n]*,\s*(?:mode\s*=\s*)?['"][rbt]*[wax+]|\.(?:write_text|write_bytes)\(|\b(?:writeFileSync|writeFile|appendFileSync|appendFile|createWriteStream)\(|\b(?:File|IO)\.(?:write|binwrite)\(|\bFile\.open\([^\n]*['"][rb]*[wa+]['"]|\bshutil\.(?:copy\w*|move)\(|\bos\.(?:replace|rename)\(|\bfs\.(?:copyFileSync|renameSync)\("#).unwrap()
    });
    let open_write = OPEN_WRITE.get_or_init(|| Regex::new(r#"\bopen\([^\n]*,\s*(?:mode\s*=\s*)?['"][rbt]*[wax+]"#).unwrap());
    let bare_write = BARE_WRITE.get_or_init(|| Regex::new(r"(\w+)\s*\.write\(").unwrap());
    let lit_re = LIT.get_or_init(|| Regex::new(lit).unwrap());

    let grab = |c: &regex::Captures| -> Option<String> {
        (1..c.len()).rev().find_map(|i| c.get(i)).map(|m| m.as_str().to_string())
    };

    let mut out = Vec::new();
    let mut precise_spans = Vec::new();
    for re in precise {
        for c in re.captures_iter(code) {
            precise_spans.push(c.get(0).unwrap().range());
            out.extend(grab(&c));
        }
    }
    // A write call that no precise pattern accounted for.
    let mut unattributed = any_write
        .find_iter(code)
        .any(|m| !precise_spans.iter().any(|s| s.start <= m.start() && m.start() < s.end));
    // A bare `.write(` on a handle counts only where no open-for-write is in
    // sight (that open already says where it writes) and the handle is not
    // stdout/stderr.
    if !unattributed && !open_write.is_match(code) {
        unattributed = bare_write
            .captures_iter(code)
            .any(|c| !matches!(&c[1], "stdout" | "stderr" | "sys" | "console" | "res" | "response"));
    }
    if unattributed {
        for c in lit_re.captures_iter(code) {
            if let Some(s) = grab(&c) {
                if looks_like_path(&s) {
                    out.push(s);
                }
            }
        }
        out.extend(argv.iter().filter(|a| !a.starts_with('-') && looks_like_path(a)).cloned());
    }
    out
}

fn looks_like_path(s: &str) -> bool {
    if s.len() > 300 || s.contains("://") || s.contains(char::is_whitespace) || s.contains('{') || s.contains('%') {
        return false;
    }
    if s.contains('/') {
        return true;
    }
    // `x.rs`, `Cargo.toml`: a name with an extension.
    match s.rsplit_once('.') {
        Some((stem, ext)) => {
            !stem.is_empty() && (1..=8).contains(&ext.len()) && ext.chars().all(|c| c.is_ascii_alphanumeric())
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CWD: &str = "/repo";

    fn targets_with(cmd: &str, script: Option<&str>) -> Vec<String> {
        let script = script.map(str::to_string);
        let read = move |_: &Path| script.clone();
        write_targets(cmd, Path::new(CWD), &read)
            .into_iter()
            .map(|p| p.display().to_string())
            .collect()
    }

    fn targets(cmd: &str) -> Vec<String> {
        targets_with(cmd, None)
    }

    /// The table: command → the paths it writes. Empty means not write-shaped.
    #[test]
    fn the_classifier_names_what_each_command_writes() {
        let cases: &[(&str, &[&str])] = &[
            // sed / perl in place
            ("sed -i s/a/b/ src/x.rs", &["/repo/src/x.rs"]),
            ("sed -i '' 's/a/b/' src/x.rs", &["/repo/src/x.rs"]),
            ("sed -i.bak -e 's/a/b/' -e 's/c/d/' src/x.rs src/y.rs", &["/repo/src/x.rs", "/repo/src/y.rs"]),
            ("sed -Ei 's/a/b/' src/x.rs", &["/repo/src/x.rs"]),
            ("sed --in-place 's/a/b/' src/x.rs", &["/repo/src/x.rs"]),
            ("sed -n '1,20p' src/x.rs", &[]),
            ("sed 's/a/b/' src/x.rs > /tmp/out", &["/tmp/out"]),
            ("perl -pi -e 's/a/b/' src/x.rs", &["/repo/src/x.rs"]),
            ("perl -i -pe 's/a/b/' src/x.rs", &["/repo/src/x.rs"]),
            ("perl -ne 'print' src/x.rs", &[]),
            // redirection and tee
            ("cat > src/x.rs <<'EOF'\nfn a() {}\nEOF", &["/repo/src/x.rs"]),
            ("cat >> src/x.rs", &["/repo/src/x.rs"]),
            ("echo hi > target/foo", &["/repo/target/foo"]),
            ("cargo test 2>&1 | tee /tmp/log", &["/tmp/log"]),
            ("echo x | tee -a src/a.rs src/b.rs", &["/repo/src/a.rs", "/repo/src/b.rs"]),
            ("cargo build 2> err.txt", &["/repo/err.txt"]),
            ("ls > /dev/null 2>&1", &[]),
            ("cmd &> out.log", &["/repo/out.log"]),
            // cp / mv / dd
            ("cp /tmp/y.rs src/x.rs", &["/repo/src/x.rs"]),
            ("cp -r a b dest/", &["/repo/dest"]),
            ("mv src/a.rs src/b.rs", &["/repo/src/b.rs", "/repo/src/a.rs"]),
            ("cp -t src/ a.rs", &["/repo/src"]),
            ("dd if=/dev/zero of=src/blob bs=1 count=1", &["/repo/src/blob"]),
            // cd within the command
            ("cd src && sed -i '' 's/a/b/' x.rs", &["/repo/src/x.rs"]),
            ("cd /elsewhere; echo x > y", &["/elsewhere/y"]),
            // interpreters
            ("python3 - <<'EOF'\nopen('src/x.rs','w').write('x')\nEOF", &["/repo/src/x.rs"]),
            ("python3 <<EOF\nimport pathlib\npathlib.Path('src/x.rs').write_text('x')\nEOF", &["/repo/src/x.rs"]),
            ("python3 -c \"open('src/x.rs','w').write('x')\"", &["/repo/src/x.rs"]),
            ("python3 -c \"print(open('src/x.rs').read())\"", &[]),
            ("node -e \"require('fs').writeFileSync('src/x.js', 'x')\"", &["/repo/src/x.js"]),
            ("ruby -e \"File.write('src/x.rb', 'x')\"", &["/repo/src/x.rb"]),
            ("python3 -m pytest tests/", &[]),
            ("node --test", &[]),
            ("node --test tests/a.test.js", &[]),
            ("python3 -c 'import sys; sys.stdout.write(\"hi\")'", &[]),
            // shells
            ("bash -c \"sed -i 's/a/b/' src/x.rs\"", &["/repo/src/x.rs"]),
            ("sh <<'EOF'\necho x > src/x.rs\nEOF", &["/repo/src/x.rs"]),
            // wrappers and hexa
            ("FOO=1 sudo sed -i s/a/b/ src/x.rs", &["/repo/src/x.rs"]),
            ("hexa do run 'x' --file src/x.rs --evidence 'cargo test' > src/log.txt", &[]),
            ("hexa loop gate 'cargo test' && sed -i s/a/b/ src/x.rs", &["/repo/src/x.rs"]),
            // read-only
            ("cargo test", &[]),
            ("git status", &[]),
            ("grep -rn 'a > b' src", &[]),
            ("npm test", &[]),
            ("echo 'x > y'", &[]),
            ("cat src/x.rs | head -5 # > nope", &[]),
            ("git diff -- src/x.rs | cat", &[]),
            ("[ -f x ] && echo $(( 1 > 0 ))", &[]),
        ];
        let mut failures = Vec::new();
        for (cmd, want) in cases {
            let got = targets(cmd);
            let want: Vec<String> = want.iter().map(|s| s.to_string()).collect();
            if got != want {
                failures.push(format!("{cmd:?}\n   want {want:?}\n    got {got:?}"));
            }
        }
        assert!(failures.is_empty(), "{} of {} cases wrong:\n{}", failures.len(), cases.len(), failures.join("\n"));
    }

    #[test]
    fn a_script_file_is_read_for_the_paths_it_writes() {
        let patch = "p = 'src/x.rs'\ns = open(p).read()\nwith open(p, 'w') as f:\n    f.write(s.replace('a', 'b'))\n";
        assert_eq!(targets_with("python3 /tmp/patch.py", Some(patch)), vec!["/repo/src/x.rs"]);
        // A script that only reads writes nothing.
        let reader = "import sys\nprint(open('src/x.rs').read())\nsys.stdout.write('done')\n";
        assert!(targets_with("python3 /tmp/read.py", Some(reader)).is_empty());
        // Writes to a path it is given: the argument is the candidate.
        let by_arg = "import sys\nfor p in sys.argv[1:]:\n    open(p, 'w').write('')\n";
        assert_eq!(targets_with("python3 /tmp/w.py src/y.rs", Some(by_arg)), vec!["/repo/src/y.rs"]);
        // A script that cannot be read is not a candidate.
        assert!(targets_with("python3 /tmp/missing.py", None).is_empty());
    }

    #[test]
    fn a_script_is_not_a_write_target_of_itself() {
        // A script that opens a variable path for writing makes every
        // path-like string in it a candidate; its own name, in a usage line, is
        // one. Running it is not editing it.
        let gen = "SOURCE = \"examples/world/gen.py\"\nUSAGE = \"python3 -I examples/world/gen.py OUTDIR\"\nwith open(path, \"w\") as f:\n    f.write(\"x\")\n";
        assert!(targets_with("python3 -I examples/world/gen.py --help", Some(gen)).is_empty());
        assert!(targets_with("python3 examples/world/gen.py --help 2>&1 | head -3", Some(gen)).is_empty());
        // Control: a path it is given is still a candidate.
        assert_eq!(targets_with("python3 examples/world/gen.py out/x.txt", Some(gen)), vec!["/repo/out/x.txt"]);
    }

    #[test]
    fn names_a_script_roots_at_an_absolute_argument_are_not_the_repos() {
        // The script is handed a directory and joins its own relative names onto
        // it (root = Path(sys.argv[1]); (root / f).write_text(...)). The names
        // are relative to that directory, not to where the command runs.
        let script = "import sys, pathlib\nroot = pathlib.Path(sys.argv[1])\nfiles = ['ci/check.sh', 'docs/operating.md', '.forgejo/workflows/ci.yml']\nfor f in files:\n    p = root / f\n    p.write_text(p.read_text().replace('a', 'b'))\n";
        let got = targets_with("python3 -I /tmp/rename.py /home/x/.cache/scratch/after", Some(script));
        assert!(got.iter().all(|p| !p.starts_with("/repo/")), "names rooted at the argument were read as the repo's: {got:?}");
        // The directory it was handed is still a candidate.
        assert!(got.contains(&"/home/x/.cache/scratch/after".to_string()), "{got:?}");
        // Control: with a relative argument, or none, the names are the cwd's.
        let got = targets_with("python3 -I /tmp/rename.py", Some(script));
        assert!(got.contains(&"/repo/ci/check.sh".to_string()), "{got:?}");
        let got = targets_with("python3 -I /tmp/rename.py out", Some(script));
        assert!(got.contains(&"/repo/ci/check.sh".to_string()), "{got:?}");
        // Control: a script that does not read its arguments keeps the cwd.
        let patch = "p = 'src/x.rs'\nopen(p, 'w').write('x')\n";
        assert_eq!(targets_with("python3 /tmp/patch.py /home/x/elsewhere", Some(patch)), vec!["/repo/src/x.rs", "/home/x/elsewhere"]);
    }

    #[test]
    fn a_heredoc_body_is_not_read_as_commands() {
        // The body is data for cat, not a `>` into src/.
        assert_eq!(targets("cat > /tmp/notes <<'EOF'\necho x > src/x.rs\nEOF"), vec!["/tmp/notes"]);
        // And the command after the heredoc is read.
        assert_eq!(targets("cat <<EOF\nhi\nEOF\nsed -i s/a/b/ src/x.rs"), vec!["/repo/src/x.rs"]);
    }

    #[test]
    fn the_script_read_is_bounded() {
        let big = format!("{}open('src/x.rs','w')", "#".repeat(SCRIPT_READ_CAP + 10));
        assert!(targets_with("python3 /tmp/big.py", Some(&big)).is_empty());
    }
}
