//! Local persistence of the session hand history.
//!
//! # Policy
//!
//! * **Location** is chosen by the caller: [`SessionStore::open`] takes a
//!   path. [`default_path`] (real CLI use only) is
//!   `$POKERBOT_HOME/session-history.txt` if `POKERBOT_HOME` is set and non-empty,
//!   otherwise `$HOME/.pokerbot/session-history.txt` (on Windows `%USERPROFILE%`).
//! * **Only records are stored** (`pokerbot-history v1`, see
//!   [`crate::history`]); statistics are recomputed on load, so they survive
//!   a restart exactly.
//! * **Writes are atomic**: the whole file is written to a sibling temp file
//!   (`<name>.tmp-<pid>`), flushed to disk, then renamed over the target.
//! * **Missing file**: loads as an empty session with
//!   [`LoadNotice::Missing`] (informational, not an error).
//! * **Corrupt, truncated, oversized or unsupported-version file**: `load`
//!   returns a typed [`StoreError`] and leaves the file untouched. Nothing is
//!   silently discarded. The file is moved aside to `<name>.corrupt-<n>`
//!   only when the caller asks, via [`SessionStore::quarantine`] or
//!   [`SessionRecorder::open_or_quarantine`].
//! * **Reset is explicit**: [`SessionStore::reset`] deletes only the store
//!   file itself, and refuses ([`StoreError::NotPokerbotFile`]) if the file is
//!   non-empty and does not start with the `pokerbot-history ` header. It never
//!   touches the directory or any other file.

use std::fmt;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use pokerbot_core::action::Seat;
use pokerbot_core::state::HandState;

use crate::history::{self, HISTORY_HEADER_PREFIX, HandRecord, HistoryError, OpponentLabel};
use crate::stats::SessionStats;

/// File name used inside the Pokerbot home directory.
pub const HISTORY_FILE_NAME: &str = "session-history.txt";
/// Largest history file `load` accepts.
pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
/// Upper bound on `.corrupt-<n>` suffixes tried.
const MAX_QUARANTINE_SLOTS: u32 = 10_000;

/// The default history path for interactive use (see module docs). `None`
/// if neither `POKERBOT_HOME` nor a home directory is available.
pub fn default_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    default_path_from(std::env::var_os("POKERBOT_HOME"), home)
}

/// Pure form of [`default_path`] for testing without touching the
/// environment.
pub fn default_path_from(
    pokerbot_home: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> Option<PathBuf> {
    if let Some(m) = pokerbot_home.filter(|m| !m.is_empty()) {
        return Some(PathBuf::from(m).join(HISTORY_FILE_NAME));
    }
    home.filter(|h| !h.is_empty())
        .map(|h| PathBuf::from(h).join(".pokerbot").join(HISTORY_FILE_NAME))
}

/// Errors from the store. I/O errors are flattened so the type is `Clone`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreError {
    /// An operating-system error.
    Io {
        op: &'static str,
        path: PathBuf,
        kind: io::ErrorKind,
        message: String,
    },
    /// The file exists but is not a valid history (corrupt or truncated).
    Corrupt { path: PathBuf, error: HistoryError },
    /// The file is a Pokerbot history of an unsupported version.
    UnsupportedVersion { path: PathBuf, version: String },
    /// The file is larger than [`MAX_FILE_BYTES`].
    TooLarge { path: PathBuf, bytes: u64 },
    /// Reset refused: the file does not look like a Pokerbot history.
    NotPokerbotFile { path: PathBuf },
}

impl StoreError {
    fn io(op: &'static str, path: &Path, e: io::Error) -> StoreError {
        StoreError::Io {
            op,
            path: path.to_path_buf(),
            kind: e.kind(),
            message: e.to_string(),
        }
    }

    /// Whether this error is about the file's contents (and so the file can
    /// be quarantined), as opposed to an I/O failure.
    pub fn is_bad_file(&self) -> bool {
        matches!(
            self,
            StoreError::Corrupt { .. }
                | StoreError::UnsupportedVersion { .. }
                | StoreError::TooLarge { .. }
        )
    }
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Io {
                op, path, message, ..
            } => write!(f, "could not {op} {}: {message}", path.display()),
            StoreError::Corrupt { path, error } => {
                write!(f, "history file {} is corrupt: {error}", path.display())
            }
            StoreError::UnsupportedVersion { path, version } => write!(
                f,
                "history file {} has unsupported version {version:?}",
                path.display()
            ),
            StoreError::TooLarge { path, bytes } => write!(
                f,
                "history file {} is too large ({bytes} bytes)",
                path.display()
            ),
            StoreError::NotPokerbotFile { path } => write!(
                f,
                "refusing to reset {}: it is not a Pokerbot history file",
                path.display()
            ),
        }
    }
}

impl std::error::Error for StoreError {}

/// Informational result of loading.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadNotice {
    /// No file yet: starting an empty session.
    Missing,
    /// Loaded this many hands.
    Loaded { hands: usize },
    /// The previous file was unreadable and was moved aside on request; a
    /// fresh session starts.
    Quarantined {
        moved_to: PathBuf,
        error: StoreError,
    },
    /// No persistence configured (in-memory session).
    Ephemeral,
}

impl fmt::Display for LoadNotice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadNotice::Missing => write!(f, "no saved history found; starting a new session"),
            LoadNotice::Loaded { hands } => write!(f, "loaded {hands} saved hands"),
            LoadNotice::Quarantined { moved_to, error } => write!(
                f,
                "{error}; it was moved to {} and a new session started",
                moved_to.display()
            ),
            LoadNotice::Ephemeral => write!(f, "history is not being saved this session"),
        }
    }
}

/// What [`SessionStore::reset`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResetOutcome {
    Deleted,
    NothingToDelete,
}

/// A history file at a caller-chosen path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionStore {
    path: PathBuf,
}

impl SessionStore {
    /// A store at `path`. Does no I/O.
    pub fn open(path: impl Into<PathBuf>) -> SessionStore {
        SessionStore { path: path.into() }
    }

    /// The history file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Loads all records (see the module policy).
    pub fn load(&self) -> Result<(Vec<HandRecord>, LoadNotice), StoreError> {
        let p = &self.path;
        let meta = match fs::metadata(p) {
            Ok(m) => m,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Ok((Vec::new(), LoadNotice::Missing));
            }
            Err(e) => return Err(StoreError::io("inspect", p, e)),
        };
        if meta.len() > MAX_FILE_BYTES {
            return Err(StoreError::TooLarge {
                path: p.clone(),
                bytes: meta.len(),
            });
        }
        let bytes = fs::read(p).map_err(|e| StoreError::io("read", p, e))?;
        let text = String::from_utf8(bytes).map_err(|_| StoreError::Corrupt {
            path: p.clone(),
            error: HistoryError::Parse {
                line: 0,
                reason: "file is not valid UTF-8".into(),
            },
        })?;
        match history::parse(&text) {
            Ok(records) => {
                let hands = records.len();
                Ok((records, LoadNotice::Loaded { hands }))
            }
            Err(HistoryError::UnsupportedVersion(version)) => Err(StoreError::UnsupportedVersion {
                path: p.clone(),
                version,
            }),
            Err(error) => Err(StoreError::Corrupt {
                path: p.clone(),
                error,
            }),
        }
    }

    /// Atomically replaces the file with `records`.
    pub fn save(&self, records: &[HandRecord]) -> Result<(), StoreError> {
        let p = &self.path;
        if let Some(dir) = p.parent().filter(|d| !d.as_os_str().is_empty()) {
            fs::create_dir_all(dir).map_err(|e| StoreError::io("create directory", dir, e))?;
        }
        let tmp = self.sibling(&format!("tmp-{}", std::process::id()));
        let text = history::to_text(records);
        let write = || -> io::Result<()> {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(text.as_bytes())?;
            f.sync_all()?;
            Ok(())
        };
        if let Err(e) = write() {
            let _ = fs::remove_file(&tmp);
            return Err(StoreError::io("write", &tmp, e));
        }
        if let Err(e) = fs::rename(&tmp, p) {
            let _ = fs::remove_file(&tmp);
            return Err(StoreError::io("replace", p, e));
        }
        Ok(())
    }

    /// Moves the current file to `<name>.corrupt-<n>` (smallest unused `n`
    /// from 1). Returns the new path, or `None` if there was no file.
    pub fn quarantine(&self) -> Result<Option<PathBuf>, StoreError> {
        if !self.exists()? {
            return Ok(None);
        }
        for n in 1..=MAX_QUARANTINE_SLOTS {
            let dest = self.sibling(&format!("corrupt-{n}"));
            if fs::symlink_metadata(&dest).is_ok() {
                continue;
            }
            fs::rename(&self.path, &dest)
                .map_err(|e| StoreError::io("move aside", &self.path, e))?;
            return Ok(Some(dest));
        }
        Err(StoreError::Io {
            op: "move aside",
            path: self.path.clone(),
            kind: io::ErrorKind::AlreadyExists,
            message: "too many quarantined files".into(),
        })
    }

    /// Deletes the history file (see the module policy).
    pub fn reset(&self) -> Result<ResetOutcome, StoreError> {
        if !self.exists()? {
            return Ok(ResetOutcome::NothingToDelete);
        }
        let mut head = Vec::new();
        {
            use std::io::Read as _;
            let f =
                fs::File::open(&self.path).map_err(|e| StoreError::io("read", &self.path, e))?;
            f.take(HISTORY_HEADER_PREFIX.len() as u64)
                .read_to_end(&mut head)
                .map_err(|e| StoreError::io("read", &self.path, e))?;
        }
        if !head.is_empty() && head != HISTORY_HEADER_PREFIX.as_bytes() {
            return Err(StoreError::NotPokerbotFile {
                path: self.path.clone(),
            });
        }
        fs::remove_file(&self.path).map_err(|e| StoreError::io("delete", &self.path, e))?;
        Ok(ResetOutcome::Deleted)
    }

    fn exists(&self) -> Result<bool, StoreError> {
        match fs::symlink_metadata(&self.path) {
            Ok(m) if m.is_file() => Ok(true),
            Ok(_) => Err(StoreError::Io {
                op: "use",
                path: self.path.clone(),
                kind: io::ErrorKind::InvalidInput,
                message: "not a regular file".into(),
            }),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(StoreError::io("inspect", &self.path, e)),
        }
    }

    fn sibling(&self, suffix: &str) -> PathBuf {
        let mut name = self
            .path
            .file_name()
            .map(|n| n.to_os_string())
            .unwrap_or_else(|| HISTORY_FILE_NAME.into());
        name.push(".");
        name.push(suffix);
        self.path.with_file_name(name)
    }
}

/// The CLI-facing session object: records hands from the human's view,
/// keeps statistics current, and persists on request.
#[derive(Clone, Debug)]
pub struct SessionRecorder {
    store: Option<SessionStore>,
    records: Vec<HandRecord>,
    stats: SessionStats,
}

impl SessionRecorder {
    /// Opens `store` and loads it. A corrupt file is an error and is left in
    /// place (see [`SessionRecorder::open_or_quarantine`]).
    pub fn open(store: SessionStore) -> Result<(SessionRecorder, LoadNotice), StoreError> {
        let (records, notice) = store.load()?;
        let stats = SessionStats::from_records(&records);
        Ok((
            SessionRecorder {
                store: Some(store),
                records,
                stats,
            },
            notice,
        ))
    }

    /// Like [`SessionRecorder::open`], but if the file is unreadable
    /// (corrupt/truncated/unsupported/too large) moves it aside to
    /// `.corrupt-<n>` and starts empty, reporting
    /// [`LoadNotice::Quarantined`]. I/O errors are still returned.
    pub fn open_or_quarantine(
        store: SessionStore,
    ) -> Result<(SessionRecorder, LoadNotice), StoreError> {
        match SessionRecorder::open(store.clone()) {
            Err(error) if error.is_bad_file() => {
                let moved_to = store.quarantine()?.unwrap_or_else(|| store.path.clone());
                Ok((
                    SessionRecorder {
                        store: Some(store),
                        records: Vec::new(),
                        stats: SessionStats::new(),
                    },
                    LoadNotice::Quarantined { moved_to, error },
                ))
            }
            other => other,
        }
    }

    /// An in-memory session that is never written to disk.
    pub fn ephemeral() -> SessionRecorder {
        SessionRecorder {
            store: None,
            records: Vec::new(),
            stats: SessionStats::new(),
        }
    }

    /// Records a finished hand from `human`'s point of view (events are
    /// filtered internally with `Viewer::Seat(human)`). Does not save.
    pub fn record_hand(
        &mut self,
        state: &HandState,
        human: Seat,
        opponent: &OpponentLabel,
    ) -> Result<&HandRecord, HistoryError> {
        let n = self.records.last().map_or(1, |r| r.hand_number + 1);
        let rec = HandRecord::from_state(n, state, human, opponent.clone())?;
        self.stats.add(&rec);
        self.records.push(rec);
        Ok(self.records.last().expect("just pushed"))
    }

    /// Current statistics.
    pub fn stats(&self) -> &SessionStats {
        &self.stats
    }

    /// All recorded hands, oldest first.
    pub fn records(&self) -> &[HandRecord] {
        &self.records
    }

    /// The store, if persistence is configured.
    pub fn store(&self) -> Option<&SessionStore> {
        self.store.as_ref()
    }

    /// Atomically writes all records. No-op for an ephemeral session.
    pub fn save(&self) -> Result<(), StoreError> {
        match &self.store {
            Some(s) => s.save(&self.records),
            None => Ok(()),
        }
    }

    /// Discards in-memory state and reloads from disk. On error the
    /// in-memory session is unchanged.
    pub fn load(&mut self) -> Result<LoadNotice, StoreError> {
        let Some(store) = &self.store else {
            return Ok(LoadNotice::Ephemeral);
        };
        let (records, notice) = store.load()?;
        self.stats = SessionStats::from_records(&records);
        self.records = records;
        Ok(notice)
    }

    /// Explicit reset: deletes the history file (only Pokerbot's own) and clears
    /// the in-memory session. On error nothing in memory changes.
    pub fn reset(&mut self) -> Result<ResetOutcome, StoreError> {
        let outcome = match &self.store {
            Some(s) => s.reset()?,
            None => ResetOutcome::NothingToDelete,
        };
        self.records.clear();
        self.stats = SessionStats::new();
        Ok(outcome)
    }
}
