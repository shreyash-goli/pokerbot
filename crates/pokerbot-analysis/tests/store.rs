//! Persistence: restart survival, missing/corrupt files, explicit reset.
//! Every test uses a unique directory under the system temp dir.

mod common;

use std::fs;

use common::*;
use pokerbot_analysis::store::{HISTORY_FILE_NAME, default_path_from};
use pokerbot_analysis::{LoadNotice, ResetOutcome, SessionRecorder, SessionStore, StoreError};
use pokerbot_core::action::Seat;
use pokerbot_core::rng::PokerbotRng;
use pokerbot_core::state::HandState;

fn play_session(rec: &mut SessionRecorder, seed: u64, hands: u64) {
    let mut rng = PokerbotRng::seed_from_u64(seed);
    for n in 0..hands {
        let button = Seat::ALL[(n % 2) as usize];
        let st = HandState::from_seed(config(button), rng.next_u64()).unwrap();
        let st = play_random(st, &mut rng);
        rec.record_hand(&st, HUMAN, &cfr_label()).unwrap();
    }
}

#[test]
fn missing_file_starts_empty_with_notice() {
    let dir = temp_dir("missing");
    let (r, notice) = SessionRecorder::open(SessionStore::open(dir.join("h.txt"))).unwrap();
    assert_eq!(notice, LoadNotice::Missing);
    assert!(r.records().is_empty());
    assert_eq!(r.stats().hands(), 0);
    assert!(!dir.join("h.txt").exists(), "open does not create files");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn stats_survive_restart_and_appends() {
    let dir = temp_dir("restart");
    let path = dir.join("nested").join("h.txt");
    let (mut r, _) = SessionRecorder::open(SessionStore::open(&path)).unwrap();
    play_session(&mut r, 1, 40);
    r.save().unwrap();
    let (stats, records) = (*r.stats(), r.records().to_vec());
    drop(r);

    let (mut r2, notice) = SessionRecorder::open(SessionStore::open(&path)).unwrap();
    assert_eq!(notice, LoadNotice::Loaded { hands: 40 });
    assert_eq!(r2.records(), &records[..]);
    assert_eq!(*r2.stats(), stats);

    // Append after restart; numbering continues.
    play_session(&mut r2, 2, 10);
    assert_eq!(r2.records().last().unwrap().hand_number, 50);
    r2.save().unwrap();
    let all = r2.records().to_vec();
    let (r3, _) = SessionRecorder::open(SessionStore::open(&path)).unwrap();
    assert_eq!(r3.records(), &all[..]);
    assert_eq!(*r3.stats(), *r2.stats());
    assert_eq!(
        r3.stats().human.overall.net_chips + r3.stats().opponent.overall.net_chips,
        0
    );
    // No temp files left behind by atomic writes.
    let names: Vec<_> = fs::read_dir(path.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, vec![std::ffi::OsString::from("h.txt")]);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn reload_discards_unsaved_hands() {
    let dir = temp_dir("reload");
    let (mut r, _) = SessionRecorder::open(SessionStore::open(dir.join("h.txt"))).unwrap();
    play_session(&mut r, 3, 5);
    r.save().unwrap();
    play_session(&mut r, 4, 3);
    assert_eq!(r.records().len(), 8);
    assert_eq!(r.load().unwrap(), LoadNotice::Loaded { hands: 5 });
    assert_eq!(r.stats().hands(), 5);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn corrupt_file_is_reported_not_lost() {
    let dir = temp_dir("corrupt");
    let path = dir.join("h.txt");
    let (mut r, _) = SessionRecorder::open(SessionStore::open(&path)).unwrap();
    play_session(&mut r, 5, 6);
    r.save().unwrap();
    let good = fs::read_to_string(&path).unwrap();
    let cases: Vec<(String, &str)> = vec![
        (good.replacen("payoffs", "payofs", 1), "corrupt"),
        (good[..good.len() / 2].to_string(), "truncated"),
        (good.replacen("v1", "v9", 1), "version"),
        (String::from("\u{0}\u{1}binary"), "garbage"),
    ];
    for (content, what) in cases {
        fs::write(&path, &content).unwrap();
        let err = SessionRecorder::open(SessionStore::open(&path)).unwrap_err();
        assert!(err.is_bad_file(), "{what}: {err}");
        if what == "version" {
            assert!(
                matches!(err, StoreError::UnsupportedVersion { ref version, .. } if version == "v9")
            );
        } else {
            assert!(matches!(err, StoreError::Corrupt { .. }), "{what}: {err:?}");
        }
        // The file is untouched by a failed load.
        assert_eq!(fs::read_to_string(&path).unwrap(), content, "{what}");
    }
    fs::write(&path, [0xffu8, 0xfe, 0x00]).unwrap();
    assert!(matches!(
        SessionStore::open(&path).load(),
        Err(StoreError::Corrupt { .. })
    ));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn quarantine_only_on_request() {
    let dir = temp_dir("quarantine");
    let path = dir.join("h.txt");
    fs::write(&path, "pokerbot-history v1\nhand 1\nbroken\n").unwrap();
    let (r, notice) = SessionRecorder::open_or_quarantine(SessionStore::open(&path)).unwrap();
    let moved = match notice {
        LoadNotice::Quarantined { moved_to, error } => {
            assert!(error.is_bad_file());
            moved_to
        }
        other => panic!("{other:?}"),
    };
    assert_eq!(moved, dir.join("h.txt.corrupt-1"));
    assert_eq!(
        fs::read_to_string(&moved).unwrap(),
        "pokerbot-history v1\nhand 1\nbroken\n",
        "bad data preserved"
    );
    assert!(!path.exists());
    assert!(r.records().is_empty());
    // Second quarantine picks the next free slot.
    fs::write(&path, "junk").unwrap();
    assert_eq!(
        SessionStore::open(&path).quarantine().unwrap(),
        Some(dir.join("h.txt.corrupt-2"))
    );
    assert_eq!(SessionStore::open(&path).quarantine().unwrap(), None);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn reset_is_explicit_and_only_touches_pokerbot_file() {
    let dir = temp_dir("reset");
    let path = dir.join("h.txt");
    let other = dir.join("unrelated.txt");
    fs::write(&other, "keep me").unwrap();
    let (mut r, _) = SessionRecorder::open(SessionStore::open(&path)).unwrap();
    play_session(&mut r, 6, 4);
    r.save().unwrap();
    // Reopening does not reset.
    let (mut r, _) = SessionRecorder::open(SessionStore::open(&path)).unwrap();
    assert_eq!(r.records().len(), 4);
    assert_eq!(r.reset().unwrap(), ResetOutcome::Deleted);
    assert!(r.records().is_empty() && r.stats().hands() == 0);
    assert!(!path.exists());
    assert_eq!(fs::read_to_string(&other).unwrap(), "keep me");
    assert_eq!(r.reset().unwrap(), ResetOutcome::NothingToDelete);
    let (_, notice) = SessionRecorder::open(SessionStore::open(&path)).unwrap();
    assert_eq!(notice, LoadNotice::Missing);

    // A store pointed at a non-Pokerbot file refuses to delete it.
    let store = SessionStore::open(&other);
    assert!(matches!(
        store.reset(),
        Err(StoreError::NotPokerbotFile { .. })
    ));
    assert_eq!(fs::read_to_string(&other).unwrap(), "keep me");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn ephemeral_session_never_writes() {
    let mut r = SessionRecorder::ephemeral();
    play_session(&mut r, 7, 3);
    r.save().unwrap();
    assert_eq!(r.load().unwrap(), LoadNotice::Ephemeral);
    assert!(r.store().is_none());
}

#[test]
fn default_path_resolution_is_pure() {
    let p = default_path_from(Some("/x/mh".into()), Some("/home/u".into()));
    assert_eq!(
        p,
        Some(std::path::PathBuf::from("/x/mh").join(HISTORY_FILE_NAME))
    );
    let p = default_path_from(Some("".into()), Some("/home/u".into()));
    assert_eq!(
        p,
        Some(std::path::PathBuf::from("/home/u/.pokerbot").join(HISTORY_FILE_NAME))
    );
    assert_eq!(default_path_from(None, None), None);
}
