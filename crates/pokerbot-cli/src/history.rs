//! Opening, describing and resetting the saved hand history.
//!
//! The storage itself is `pokerbot_analysis::store`. This module turns the
//! command-line choice (`--history <path>`, `--no-history`, default) into a
//! [`SessionRecorder`] plus short notices for the session header. It never
//! fails: an unusable history degrades to an in-memory session with a
//! warning. A corrupt or unsupported file is moved aside to
//! `<name>.corrupt-<n>` (contents kept) and a fresh history starts.

use std::path::PathBuf;

use pokerbot_analysis::store::{self, LoadNotice, ResetOutcome, SessionStore};
use pokerbot_analysis::{SessionRecorder, StoreError};

use crate::args::HistoryChoice;

/// The history file for `choice`; `default` supplies the default location
/// (`pokerbot_analysis::store::default_path` in the binary). `None` means no
/// persistence.
pub fn resolve_path(
    choice: &HistoryChoice,
    default: impl FnOnce() -> Option<PathBuf>,
) -> Option<PathBuf> {
    match choice {
        HistoryChoice::Disabled => None,
        HistoryChoice::Path(p) => Some(PathBuf::from(p)),
        HistoryChoice::Default => default(),
    }
}

/// Opens the history for `choice`. Returns the recorder and the notices to
/// show in the session header.
pub fn open(
    choice: &HistoryChoice,
    default: impl FnOnce() -> Option<PathBuf>,
) -> (SessionRecorder, Vec<String>) {
    let Some(path) = resolve_path(choice, default) else {
        let why = match choice {
            HistoryChoice::Disabled => "History: not saved (--no-history).".to_string(),
            _ => "History: no home directory found, so nothing is saved \
                  (set POKERBOT_HOME or pass --history <path>)."
                .to_string(),
        };
        return (SessionRecorder::ephemeral(), vec![why]);
    };
    let shown = path.display().to_string();
    match SessionRecorder::open_or_quarantine(SessionStore::open(&path)) {
        Ok((rec, notice)) => {
            let line = match notice {
                LoadNotice::Missing => {
                    format!("History: no saved hands at {shown}; starting a new history.")
                }
                LoadNotice::Loaded { hands } => {
                    format!("History: {hands} saved hand(s) loaded from {shown}.")
                }
                LoadNotice::Quarantined { moved_to, error } => format!(
                    "History WARNING: {error}. The unreadable file was moved to {} \
                     (contents kept); starting a new history at {shown}.",
                    moved_to.display()
                ),
                LoadNotice::Ephemeral => "History: not saved.".to_string(),
            };
            (rec, vec![line])
        }
        Err(e) => (
            SessionRecorder::ephemeral(),
            vec![format!(
                "History WARNING: {e}. This session's hands will not be saved."
            )],
        ),
    }
}

/// Carries out `--reset-stats`. Returns the message and whether it
/// succeeded (the binary exits 0 or 1).
pub fn reset(choice: &HistoryChoice, default: impl FnOnce() -> Option<PathBuf>) -> (String, bool) {
    let Some(path) = resolve_path(choice, default) else {
        return (
            "no history location (set POKERBOT_HOME or pass --history <path>); nothing deleted"
                .to_string(),
            false,
        );
    };
    let shown = path.display().to_string();
    match SessionStore::open(&path).reset() {
        Ok(ResetOutcome::Deleted) => (format!("Deleted hand history {shown}."), true),
        Ok(ResetOutcome::NothingToDelete) => (
            format!("No hand history at {shown}; nothing deleted."),
            true,
        ),
        Err(e @ StoreError::NotPokerbotFile { .. }) => (format!("{e}; nothing deleted"), false),
        Err(e) => (format!("{e}; nothing deleted"), false),
    }
}

/// The default location used by the binary.
pub fn default_path() -> Option<PathBuf> {
    store::default_path()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "pokerbot-cli-history-unit-{}-{tag}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn disabled_and_missing_default_are_ephemeral() {
        let (r, n) = open(&HistoryChoice::Disabled, || panic!("not consulted"));
        assert!(r.store().is_none());
        assert!(n[0].contains("--no-history"));
        let (r, n) = open(&HistoryChoice::Default, || None);
        assert!(r.store().is_none());
        assert!(n[0].contains("POKERBOT_HOME"));
    }

    #[test]
    fn default_is_only_used_for_default_choice() {
        let d = tmp("default");
        let p = d.join("h.txt");
        let (r, n) = open(&HistoryChoice::Default, || Some(p.clone()));
        assert_eq!(r.store().unwrap().path(), p);
        assert!(n[0].contains("starting a new history"), "{n:?}");
        let explicit = HistoryChoice::Path(p.display().to_string());
        assert_eq!(
            resolve_path(&explicit, || panic!("not consulted")),
            Some(p.clone())
        );
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn corrupt_is_quarantined_and_reset_refuses_foreign_files() {
        let d = tmp("corrupt");
        let p = d.join("h.txt");
        std::fs::write(&p, "pokerbot-history v1\nhand 1\n").unwrap();
        let choice = HistoryChoice::Path(p.display().to_string());
        let (r, n) = open(&choice, || None);
        assert!(r.records().is_empty() && r.store().is_some());
        assert!(
            n[0].contains("WARNING") && n[0].contains("corrupt-1"),
            "{n:?}"
        );
        assert!(d.join("h.txt.corrupt-1").exists());

        let foreign = d.join("notes.txt");
        std::fs::write(&foreign, "my notes").unwrap();
        let (msg, ok) = reset(&HistoryChoice::Path(foreign.display().to_string()), || None);
        assert!(!ok && msg.contains("not a Pokerbot history"), "{msg}");
        assert_eq!(std::fs::read_to_string(&foreign).unwrap(), "my notes");
        let (msg, ok) = reset(&choice, || None);
        assert!(ok && msg.contains("nothing deleted"), "{msg}");
        std::fs::remove_dir_all(d).unwrap();
    }
}
