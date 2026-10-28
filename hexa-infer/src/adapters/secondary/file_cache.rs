//! Tool-less completion replies kept on disk (ADR-2610281516 §2): one file
//! per key under `~/.hexa/cache/completions/`, holding the reply's text.

use std::path::PathBuf;

use crate::ports::CompletionCache;

pub struct FileCompletionCache {
    dir: PathBuf,
}

impl FileCompletionCache {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }
}

impl CompletionCache for FileCompletionCache {
    fn get(&self, key: &str) -> Option<String> {
        std::fs::read_to_string(self.dir.join(key)).ok()
    }

    fn put(&self, key: &str, reply: &str) {
        if std::fs::create_dir_all(&self.dir).is_err() {
            return;
        }
        // Written whole, then renamed: a reader never sees half a reply.
        let tmp = self.dir.join(format!(".{key}.{}.tmp", std::process::id()));
        if std::fs::write(&tmp, reply).is_ok() {
            let _ = std::fs::rename(&tmp, self.dir.join(key));
        }
    }
}
