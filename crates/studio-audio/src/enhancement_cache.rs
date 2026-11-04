//! Disk cache of enhanced (denoised) lane audio. Entries are keyed by task, enhancer model
//! and the SHA-256 of each lane's original WAV bytes, so reopening a task reuses its
//! enhancement without rerunning the model, and changed audio or a new model misses.
//!
//! Layout: `<root>/<sha256(task id)>/<model id>/<source sha256>.wav`. Each task keeps only
//! its latest stored set; the least recently stored tasks beyond the cap are removed.

use std::{
    fs, io,
    path::{Path, PathBuf},
    time::SystemTime,
};

use sha2::{Digest, Sha256};

/// Tasks whose enhanced audio is kept on disk.
pub const MAX_CACHED_ENHANCEMENT_TASKS: usize = 24;

/// Lowercase hex SHA-256 of `bytes` (the source digest of an original WAV).
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Clone, Debug)]
pub struct EnhancedAudioCache {
    root: PathBuf,
    max_tasks: usize,
}

impl EnhancedAudioCache {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            max_tasks: MAX_CACHED_ENHANCEMENT_TASKS,
        }
    }

    /// Enhanced WAV bytes for each source digest (in order), or `None` when any is missing.
    pub fn load(
        &self,
        task_id: &str,
        model_id: &str,
        source_digests: &[&str],
    ) -> io::Result<Option<Vec<Vec<u8>>>> {
        let dir = self.model_dir(task_id, model_id)?;
        let mut lanes = Vec::with_capacity(source_digests.len());
        for digest in source_digests {
            match fs::read(dir.join(entry_file_name(digest)?)) {
                Ok(bytes) => lanes.push(bytes),
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(error),
            }
        }
        Ok(Some(lanes))
    }

    /// Stores `(source digest, enhanced WAV)` pairs as the task's only cached audio, then
    /// trims the cache to the most recently stored tasks. Files are written atomically, so a
    /// partial store is a miss, never a corrupt hit.
    pub fn store(&self, task_id: &str, model_id: &str, lanes: &[(&str, &[u8])]) -> io::Result<()> {
        let dir = self.model_dir(task_id, model_id)?;
        fs::create_dir_all(&dir)?;
        let mut keep = Vec::with_capacity(lanes.len());
        for (digest, wav) in lanes {
            let name = entry_file_name(digest)?;
            let temporary = dir.join(format!("{name}.partial"));
            fs::write(&temporary, wav)?;
            fs::rename(&temporary, dir.join(&name))?;
            keep.push(name);
        }
        let task_dir = dir.parent().expect("model dir is inside the task dir");
        remove_other_entries(task_dir, dir.file_name(), None)?;
        remove_other_entries(&dir, None, Some(&keep))?;
        self.trim_tasks()
    }

    fn model_dir(&self, task_id: &str, model_id: &str) -> io::Result<PathBuf> {
        if model_id.is_empty()
            || model_id.starts_with('.')
            || !model_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("enhancer model id `{model_id}` is not a valid cache key"),
            ));
        }
        Ok(self
            .root
            .join(sha256_hex(task_id.as_bytes()))
            .join(model_id))
    }

    /// Removes the least recently stored task directories beyond `max_tasks`.
    fn trim_tasks(&self) -> io::Result<()> {
        let mut tasks = fs::read_dir(&self.root)?
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .map(|entry| {
                let stored = newest_modification(&entry.path());
                (stored, entry.path())
            })
            .collect::<Vec<_>>();
        if tasks.len() <= self.max_tasks {
            return Ok(());
        }
        tasks.sort_by_key(|(stored, _)| std::cmp::Reverse(*stored));
        for (_, path) in tasks.into_iter().skip(self.max_tasks) {
            fs::remove_dir_all(path)?;
        }
        Ok(())
    }
}

fn entry_file_name(digest: &str) -> io::Result<String> {
    if digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(format!("{}.wav", digest.to_ascii_lowercase()))
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "source digest must be a SHA-256 hex string",
        ))
    }
}

/// Deletes the entries of `dir` except the directory `except_dir` or the files in `keep`.
fn remove_other_entries(
    dir: &Path,
    except_dir: Option<&std::ffi::OsStr>,
    keep: Option<&[String]>,
) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        if except_dir.is_some_and(|except| except == name)
            || keep.is_some_and(|keep| keep.iter().any(|kept| name == kept.as_str()))
        {
            continue;
        }
        if entry.file_type()?.is_dir() {
            fs::remove_dir_all(entry.path())?;
        } else {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

/// Latest modification time of the files under a task directory (its last store).
fn newest_modification(task_dir: &Path) -> SystemTime {
    let mut newest = SystemTime::UNIX_EPOCH;
    let Ok(models) = fs::read_dir(task_dir) else {
        return newest;
    };
    for model in models.filter_map(Result::ok) {
        let Ok(files) = fs::read_dir(model.path()) else {
            continue;
        };
        for file in files.filter_map(Result::ok) {
            if let Ok(modified) = file.metadata().and_then(|metadata| metadata.modified()) {
                newest = newest.max(modified);
            }
        }
    }
    newest
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODEL: &str = "zipenhancer-webgpu-2026-10-09-r1";

    fn digest(seed: u8) -> String {
        sha256_hex(&[seed])
    }

    #[test]
    fn stored_lanes_load_back_in_request_order() {
        let temp = tempfile::tempdir().unwrap();
        let cache = EnhancedAudioCache::new(temp.path());
        let (first, second) = (digest(1), digest(2));

        cache
            .store("task-1", MODEL, &[(&first, b"one"), (&second, b"two")])
            .unwrap();

        assert_eq!(
            cache.load("task-1", MODEL, &[&second, &first]).unwrap(),
            Some(vec![b"two".to_vec(), b"one".to_vec()])
        );
    }

    #[test]
    fn other_tasks_models_and_sources_miss() {
        let temp = tempfile::tempdir().unwrap();
        let cache = EnhancedAudioCache::new(temp.path());
        let source = digest(1);
        cache.store("task-1", MODEL, &[(&source, b"one")]).unwrap();

        assert_eq!(cache.load("task-2", MODEL, &[&source]).unwrap(), None);
        assert_eq!(
            cache.load("task-1", "other-model", &[&source]).unwrap(),
            None
        );
        assert_eq!(
            cache.load("task-1", MODEL, &[&source, &digest(2)]).unwrap(),
            None
        );
    }

    #[test]
    fn storing_replaces_the_previous_set_of_the_task() {
        let temp = tempfile::tempdir().unwrap();
        let cache = EnhancedAudioCache::new(temp.path());
        let (old, new) = (digest(1), digest(2));
        cache
            .store("task-1", "old-model", &[(&old, b"old")])
            .unwrap();

        cache.store("task-1", MODEL, &[(&new, b"new")]).unwrap();

        assert_eq!(cache.load("task-1", "old-model", &[&old]).unwrap(), None);
        assert_eq!(
            cache.load("task-1", MODEL, &[&new]).unwrap(),
            Some(vec![b"new".to_vec()])
        );
    }

    #[test]
    fn keeps_only_the_most_recently_stored_tasks() {
        let temp = tempfile::tempdir().unwrap();
        let mut cache = EnhancedAudioCache::new(temp.path());
        cache.max_tasks = 2;
        let source = digest(1);
        for task in ["task-1", "task-2", "task-3"] {
            cache.store(task, MODEL, &[(&source, b"pcm")]).unwrap();
            // Distinct modification times on coarse-grained file systems.
            std::thread::sleep(std::time::Duration::from_millis(20));
        }

        assert_eq!(cache.load("task-1", MODEL, &[&source]).unwrap(), None);
        assert!(cache.load("task-2", MODEL, &[&source]).unwrap().is_some());
        assert!(cache.load("task-3", MODEL, &[&source]).unwrap().is_some());
    }

    #[test]
    fn rejects_keys_that_could_escape_the_cache() {
        let temp = tempfile::tempdir().unwrap();
        let cache = EnhancedAudioCache::new(temp.path());

        assert!(cache.load("task", "../model", &[&digest(1)]).is_err());
        assert!(cache.load("task", MODEL, &["../../secret"]).is_err());
    }
}
