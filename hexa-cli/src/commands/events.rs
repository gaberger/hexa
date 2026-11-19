//! hexa's AG-UI producer (ADR-2611161830): the custom JSON-lines binding.
//!
//! `--events` makes a working verb report itself as an AG-UI event stream:
//! one event object per line, flushed as it happens.
//!
//! - `--events` alone: the events go to stdout and nothing else does. At
//!   startup hexa keeps a private copy of stdout for them and points fd 1 at
//!   stderr, so every other line hexa prints, and the output of any command it
//!   runs (a gate, git), goes to stderr and cannot corrupt the stream.
//! - `--events=<path>`: the events go to that file, and stdout stays the
//!   person's.
//!
//! A verb's work is one run. `start` opens it with the loop as state,
//! `step_start`/`step_end` mark its phases, and `finish` or `fail` ends it.
//! `main` ends a run a verb left open, `RUN_ERROR` with the error when the
//! verb failed, so a stream is never left without a terminal event by an
//! early return. The ordering rules are the domain's `Stream`, and a rule
//! broken here is a bug in hexa, reported on stderr, not written to the stream.

use hexa_core::ports::agui::{run_started, Event, EventSink, Outcome, Stream};
use serde_json::{Map, Value};
use std::io::Write;
use std::sync::Mutex;

struct LineSink(std::fs::File);

impl EventSink for LineSink {
    fn line(&mut self, line: &str) -> std::io::Result<()> {
        self.0.write_all(line.as_bytes())?;
        self.0.write_all(b"\n")?;
        self.0.flush()
    }
}

struct Producer {
    stream: Stream,
    sink: Box<dyn EventSink>,
}

static PRODUCER: Mutex<Option<Producer>> = Mutex::new(None);

/// Turn the stream on. `dest` is `-` for stdout, else a file path. Called
/// once, from `main`, before any verb runs.
pub fn init(dest: &str) -> anyhow::Result<()> {
    let file = if dest == "-" || dest.is_empty() {
        std::io::stdout().flush()?;
        // SAFETY: plain descriptor calls on this process's own fds 1 and 2.
        // `dup(1)` gives the stream its own descriptor for the real stdout;
        // `dup2(2, 1)` then sends everything else written to fd 1 to stderr.
        unsafe {
            let fd = libc::dup(1);
            if fd < 0 || libc::dup2(2, 1) < 0 {
                anyhow::bail!("--events: could not keep stdout for the event stream");
            }
            use std::os::fd::FromRawFd;
            std::fs::File::from_raw_fd(fd)
        }
    } else {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dest)
            .map_err(|e| anyhow::anyhow!("--events={dest}: {e}"))?
    };
    *PRODUCER.lock().unwrap() = Some(Producer { stream: Stream::new(), sink: Box::new(LineSink(file)) });
    Ok(())
}

/// Whether this process is reporting AG-UI events.
pub fn enabled() -> bool {
    PRODUCER.lock().map(|p| p.is_some()).unwrap_or(false)
}

fn with(f: impl FnOnce(&mut Producer) -> Vec<String>) {
    let mut guard = match PRODUCER.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    let Some(p) = guard.as_mut() else { return };
    for line in f(p) {
        if let Err(e) = p.sink.line(&line) {
            eprintln!("hexa: the event stream could not be written: {e}");
        }
    }
}

/// Emit one event. Nothing when events are off.
pub fn emit(event: Event) {
    with(|p| match p.stream.emit(event) {
        Ok(line) => vec![line],
        Err(v) => {
            eprintln!("hexa: bug: an AG-UI event broke the protocol and was not sent: {v}");
            Vec::new()
        }
    });
}

/// Open a run for `verb`. Its thread is the loop session, so one session's
/// runs form one thread; its run id is new. The loop, when there is one,
/// is the run's state.
///
/// `main` opens every verb's run before the verb starts, so a verb that calls
/// this joins that run rather than opening a second one.
pub fn start(cwd: &std::path::Path) {
    if !enabled() || active() {
        return;
    }
    let thread = crate::commands::loop_cmd::session_id();
    let run = uuid::Uuid::new_v4().to_string();
    emit(run_started(&thread, &run));
    let lp = crate::commands::loop_cmd::loop_summary(cwd);
    if lp.get("recorded").and_then(|r| r.as_bool()) == Some(true) {
        emit(Event::StateSnapshot { snapshot: lp, subagent_run_id: None });
    }
}

/// Whether a run is open now.
fn active() -> bool {
    PRODUCER.lock().map(|p| p.as_ref().is_some_and(|p| p.stream.active())).unwrap_or(false)
}

/// From `main`, before the verb runs: open its run, with a step named for the
/// verb (`analyze`, `loop task add`), closed when the run ends.
pub fn start_verb(cwd: &std::path::Path, verb: &str) {
    start(cwd);
    step_start(verb);
}

/// Leave the process with `code`, ending an open run first: `RUN_FINISHED`
/// on 0, else `RUN_ERROR` with code `exit_status`. A verb that calls
/// `std::process::exit` directly skips `main`, and its stream would be left
/// without a terminal event, which a consumer must read as a crash.
pub fn exit(code: i32) -> ! {
    with(|p| {
        if !p.stream.active() {
            return Vec::new();
        }
        if code == 0 {
            p.stream.finish(None, None, None)
        } else {
            p.stream.fail(&format!("hexa exited with status {code}"), Some("exit_status"), None)
        }
    });
    std::process::exit(code)
}

pub fn step_start(name: &str) {
    emit(Event::StepStarted { step_name: name.to_string(), subagent_run_id: None });
}

pub fn step_end(name: &str) {
    emit(Event::StepFinished { step_name: name.to_string(), subagent_run_id: None });
}

/// Create or replace one activity message.
pub fn activity(message_id: &str, activity_type: &str, content: Map<String, Value>) {
    emit(Event::ActivitySnapshot { message_id: message_id.to_string(), activity_type: activity_type.to_string(), content, replace: None, subagent_run_id: None });
}

/// End the run as done, with what it returns.
pub fn finish(result: Option<Value>) {
    with(|p| p.stream.finish(result, None, None));
}

/// End the run as stopped on purpose.
pub fn cancel() {
    with(|p| p.stream.finish(None, Some(Outcome::Cancelled), None));
}

/// End the run as failed. `code` is machine-readable (`gate_failed`, `refused`).
pub fn fail(code: &str, message: &str) {
    with(|p| p.stream.fail(message, Some(code), None));
}

/// From `main`, after the verb returned: a run still open is ended, as failed
/// when the verb failed and as done when it returned Ok without saying so.
pub fn close(outcome: Result<(), &str>) {
    with(|p| {
        if !p.stream.active() {
            return Vec::new();
        }
        match outcome {
            Ok(()) => p.stream.finish(None, None, None),
            Err(msg) => p.stream.fail(msg, Some("error"), None),
        }
    });
}
