//! Preflop classes, public history encoding, info keys, and the
//! hidden-information guarantee. Tests are prefixed `preflop_` so
//! `cargo test -p pokerbot-cfr preflop` selects them.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use pokerbot_cfr::abstraction::history::{HistoryKey, MAX_HISTORY_BYTES, max_history_len};
use pokerbot_cfr::abstraction::key::{
    AbstractionError, CardAbstraction, InfoKey, KeyError, PreflopClassAbstraction, fnv1a64,
};
use pokerbot_cfr::abstraction::preflop::{ClassKind, NUM_PREFLOP_CLASSES, PreflopClass};
use pokerbot_core::action::Seat;
use pokerbot_core::card::Card;
use pokerbot_core::deck::Deck;
use pokerbot_core::rng::PokerbotRng;
use pokerbot_core::rules::{ActionKind, Position, RulesConfig, StackDepth, Street};
use pokerbot_core::state::{HandConfig, HandState};

fn c(s: &str) -> Card {
    s.parse().unwrap()
}

fn cls(s: &str) -> PreflopClass {
    s.parse().unwrap()
}

// ------------------------------------------------------------------ classes

#[test]
fn preflop_all_1326_combos_map_to_169_classes() {
    let cards: Vec<Card> = Card::all().collect();
    let mut counts: BTreeMap<PreflopClass, usize> = BTreeMap::new();
    let mut total = 0;
    for i in 0..cards.len() {
        for j in i + 1..cards.len() {
            *counts
                .entry(PreflopClass::of([cards[i], cards[j]]))
                .or_default() += 1;
            total += 1;
        }
    }
    assert_eq!(total, 1326);
    assert_eq!(counts.len(), NUM_PREFLOP_CLASSES);
    let mut by_kind = [0usize; 3];
    for (class, n) in &counts {
        assert_eq!(*n, usize::from(class.combos()), "{class}");
        let k = match class.kind() {
            ClassKind::Pair => 0,
            ClassKind::Suited => 1,
            ClassKind::Offsuit => 2,
        };
        by_kind[k] += 1;
    }
    assert_eq!(by_kind, [13, 78, 78]);
    assert_eq!(13 * 6 + 78 * 4 + 78 * 12, 1326);
    let sum: usize = PreflopClass::all().map(|c| usize::from(c.combos())).sum();
    assert_eq!(sum, 1326);
}

#[test]
fn preflop_class_combo_enumeration_round_trips() {
    let mut seen = BTreeSet::new();
    for (i, class) in PreflopClass::all().enumerate() {
        assert_eq!(usize::from(class.index()), i);
        assert_eq!(PreflopClass::from_index(class.index()), Some(class));
        let combos = class.hole_combos();
        assert_eq!(combos.len(), usize::from(class.combos()));
        for h in combos {
            assert_eq!(PreflopClass::of(h), class);
            let mut k = [h[0].index(), h[1].index()];
            k.sort_unstable();
            assert!(seen.insert(k), "combo listed twice");
        }
    }
    assert_eq!(seen.len(), 1326);
    assert_eq!(PreflopClass::from_index(169), None);
}

#[test]
fn preflop_order_and_suit_invariance() {
    let cards: Vec<Card> = Card::all().collect();
    for &a in &cards {
        for &b in &cards {
            if a != b {
                assert_eq!(PreflopClass::of([a, b]), PreflopClass::of([b, a]));
            }
        }
    }
    // Relabeling suits by any permutation preserves the class.
    let perms = [[1u8, 2, 3, 0], [3, 2, 1, 0], [0, 2, 1, 3]];
    for &a in &cards {
        for &b in &cards {
            if a == b {
                continue;
            }
            for p in perms {
                let map = |x: Card| {
                    let s = p[usize::from(x.index() % 4)];
                    Card::from_index(x.index() / 4 * 4 + s).unwrap()
                };
                assert_eq!(PreflopClass::of([a, b]), PreflopClass::of([map(a), map(b)]));
            }
        }
    }
}

#[test]
fn preflop_display_fromstr_round_trip() {
    let mut names = BTreeSet::new();
    for class in PreflopClass::all() {
        let s = class.to_string();
        assert_eq!(s.parse::<PreflopClass>().unwrap(), class, "{s}");
        assert!(names.insert(s));
    }
    assert_eq!(names.len(), 169);
    for bad in ["", "A", "AAs", "AK", "AKx", "1A", "AKso", "ak"] {
        assert!(bad.parse::<PreflopClass>().is_err(), "{bad:?} parsed");
    }
}

#[test]
fn preflop_golden_values() {
    assert_eq!(cls("AA").index(), 0);
    assert_eq!(cls("KK").index(), 1);
    assert_eq!(cls("22").index(), 12);
    assert_eq!(cls("AKs").index(), 13);
    assert_eq!(cls("A2s").index(), 24);
    assert_eq!(cls("KQs").index(), 25);
    assert_eq!(cls("32s").index(), 90);
    assert_eq!(cls("AKo").index(), 91);
    assert_eq!(cls("32o").index(), 168);
    // 7 = rank 5, 2 = rank 0: 91 + (78 - 5*6/2 + (5 - 1 - 0)) = 158.
    assert_eq!(cls("72o").index(), 158);
    assert_eq!(cls("KAs"), cls("AKs"));
    assert_eq!(cls("T9o").to_string(), "T9o");
    assert_eq!(PreflopClass::of([c("As"), c("Ah")]).to_string(), "AA");
    assert_eq!(PreflopClass::of([c("Kh"), c("Ah")]).to_string(), "AKs");
    assert_eq!(PreflopClass::of([c("Ah"), c("Kh")]), cls("AKs"));
    assert_eq!(PreflopClass::of([c("2c"), c("7d")]).to_string(), "72o");
    assert_eq!(cls("AA").combos(), 6);
    assert_eq!(cls("AKs").combos(), 4);
    assert_eq!(cls("72o").combos(), 12);
}

// ------------------------------------------------------------------ hand helpers

fn config(depth: StackDepth, button: Seat) -> HandConfig {
    HandConfig {
        rules: RulesConfig::standard(depth),
        button,
    }
}

/// Random legal action; `aggro` biases toward raises to exercise long histories.
fn pick(hand: &HandState, rng: &mut PokerbotRng, aggro: bool) -> ActionKind {
    let legal = hand.legal_actions();
    if aggro && rng.gen_range(4) != 0 {
        if let Some(a) = legal
            .iter()
            .find(|a| matches!(a.kind, ActionKind::HalfPot | ActionKind::Pot))
        {
            return a.kind;
        }
    }
    legal[rng.gen_range(legal.len() as u64) as usize].kind
}

/// Plays a hand, returning action kinds and the state before each action.
fn play(
    cfg: HandConfig,
    deck: Deck,
    rng: &mut PokerbotRng,
    aggro: bool,
) -> (Vec<ActionKind>, Vec<HandState>) {
    let mut hand = HandState::new(cfg, deck).unwrap();
    let mut kinds = Vec::new();
    let mut states = Vec::new();
    while !hand.is_terminal() {
        let k = pick(&hand, rng, aggro);
        states.push(hand.clone());
        hand.apply(k).unwrap();
        kinds.push(k);
    }
    (kinds, states)
}

/// States before each action when replaying `kinds` on `deck`.
fn replay(cfg: HandConfig, deck: Deck, kinds: &[ActionKind]) -> Vec<HandState> {
    let mut hand = HandState::new(cfg, deck).unwrap();
    let mut states = Vec::new();
    for &k in kinds {
        states.push(hand.clone());
        hand.apply(k).unwrap();
    }
    assert!(hand.is_terminal());
    states
}

/// Test-only "exact" abstraction: bucket depends on every own hole card and
/// every revealed board card, so the swap test would detect any leak path
/// through the card inputs as well as through history.
struct ExactCards;

impl CardAbstraction for ExactCards {
    fn bucket(&self, _s: Street, hole: [Card; 2], board: &[Card]) -> Result<u16, AbstractionError> {
        let mut h: Vec<u8> = hole.iter().map(|c| c.index()).collect();
        h.sort_unstable();
        h.extend(board.iter().map(|c| c.index()));
        Ok((fnv1a64(&h) % 65_535) as u16)
    }
    fn num_buckets(&self, _s: Street) -> u16 {
        u16::MAX
    }
}

fn key_for(state: &HandState, abs: &dyn CardAbstraction) -> Option<(Seat, InfoKey)> {
    let seat = state.to_act()?;
    let obs = state.observation(seat);
    let street = obs.street.betting()?;
    if abs.num_buckets(street) == 0 {
        return None;
    }
    Some((seat, InfoKey::from_observation(&obs, abs).unwrap()))
}

// ------------------------------------------------------------------ history

#[test]
fn preflop_history_encoding_deterministic_injective_and_round_trips() {
    let mut rng = PokerbotRng::seed_from_u64(2024);
    // public sequence (street per action + current street) -> bytes
    let mut by_seq: HashMap<Vec<(u8, u8)>, Vec<u8>> = HashMap::new();
    let mut by_bytes: HashMap<Vec<u8>, Vec<(u8, u8)>> = HashMap::new();
    let mut max_seen = 0usize;
    for i in 0..3_000u64 {
        let depth = StackDepth::ALL[(i % 3) as usize];
        let cfg = config(depth, Seat::ALL[(i % 2) as usize]);
        let (_, states) = play(cfg, Deck::from_seed(i), &mut rng, i % 4 == 0);
        for st in &states {
            let seat = st.to_act().unwrap();
            let obs = st.observation(seat);
            let street = obs.street.betting().unwrap();
            let hk = HistoryKey::from_records(&obs.history, street).unwrap();
            let again = HistoryKey::from_records(&obs.history, street).unwrap();
            assert_eq!(hk, again);
            assert_eq!(hk.street(), street);
            assert_eq!(HistoryKey::from_bytes(hk.as_bytes()).unwrap(), hk);
            let bound = max_history_len(cfg.rules.starting_stack, cfg.rules.big_blind);
            assert!(
                hk.len() <= bound && bound <= MAX_HISTORY_BYTES,
                "{hk} > {bound}"
            );
            max_seen = max_seen.max(hk.len());

            let mut seq: Vec<(u8, u8)> = obs
                .history
                .iter()
                .map(|r| (r.street as u8, r.kind.index() as u8))
                .collect();
            seq.push((street as u8, 0xFF));
            let bytes = hk.as_bytes().to_vec();
            if let Some(prev) = by_seq.insert(seq.clone(), bytes.clone()) {
                assert_eq!(prev, bytes);
            }
            if let Some(prev) = by_bytes.insert(bytes, seq.clone()) {
                assert_eq!(prev, seq, "collision for {hk}");
            }
        }
    }
    assert!(
        by_seq.len() > 500, // observed 975 with this seed
        "only {} distinct histories",
        by_seq.len()
    );
    assert_eq!(by_seq.len(), by_bytes.len());
    eprintln!(
        "distinct histories: {}, max bytes seen: {max_seen}",
        by_seq.len()
    );
}

#[test]
fn preflop_history_display_and_bad_bytes() {
    let h = HistoryKey::from_bytes(&[2, 1, 0x0F, 3, 2, 0x0F]).unwrap();
    assert_eq!(h.to_string(), "Pre:c-k|Flop:h-c|Turn:");
    assert_eq!(h.street(), Street::Turn);
    assert_eq!(HistoryKey::from_bytes(&[]).unwrap().to_string(), "Pre:");
    assert!(HistoryKey::from_bytes(&[6]).is_err());
    assert!(HistoryKey::from_bytes(&[0x0F; 4]).is_err());
}

// ------------------------------------------------------------------ keys

#[test]
fn preflop_info_key_bytes_and_golden_hashes() {
    // Published FNV-1a 64 test vectors.
    assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
    assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
    assert_eq!(fnv1a64(b"foobar"), 0x8594_4171_f739_67e8);

    // Button, AA, first to act preflop.
    let k1 = InfoKey::from_bytes(&[1, 0, 0, 0, 0, 0]).unwrap();
    assert_eq!(k1.position(), Position::Button);
    assert_eq!(k1.bucket(), 0);
    assert_eq!(k1.to_string(), "BTN#0 Pre:");
    // BB, 72o, facing a limp.
    let k2 = InfoKey::from_bytes(&[1, 0, 1, 158, 0, 1, 2]).unwrap();
    assert_eq!(k2.to_string(), "BB#158 Pre:c");
    // Flop key (bucket from a hypothetical postflop abstraction).
    let k3 = InfoKey::from_bytes(&[1, 1, 1, 0x34, 0x12, 3, 2, 1, 0x0F]).unwrap();
    assert_eq!(k3.bucket(), 0x1234);
    assert_eq!(k3.to_string(), "BB#4660 Pre:c-k|Flop:");
    for k in [&k1, &k2, &k3] {
        assert_eq!(InfoKey::from_bytes(&k.to_bytes()).unwrap(), *k);
        assert_eq!(k.hash64(), fnv1a64(&k.to_bytes()));
    }
    let hashes = [k1.hash64(), k2.hash64(), k3.hash64()];
    eprintln!(
        "golden: {:#018x} {:#018x} {:#018x}",
        hashes[0], hashes[1], hashes[2]
    );
    assert_eq!(hashes, GOLDEN);

    // Malformed inputs.
    for bad in [
        &[][..],
        &[2, 0, 0, 0, 0, 0],    // version
        &[1, 4, 0, 0, 0, 0],    // street
        &[1, 0, 2, 0, 0, 0],    // position
        &[1, 0, 0, 0, 0, 1],    // length mismatch
        &[1, 1, 0, 0, 0, 0],    // street vs history
        &[1, 0, 0, 0, 0, 1, 9], // bad action byte
    ] {
        assert!(InfoKey::from_bytes(bad).is_err(), "{bad:?}");
    }
}

// Pinned portable FNV-1a values; changing the key layout must bump KEY_FORMAT_VERSION.
const GOLDEN: [u64; 3] = [
    0xfb4e_98c7_3bab_ab04,
    0x022d_f08f_acd5_c0ee,
    0x1d9e_e21f_84f3_294d,
];

#[test]
fn preflop_key_from_observation_live_hand() {
    let cfg = config(StackDepth::Bb100, Seat::Zero);
    let deck = Deck::from_order(
        ["Kd", "As", "Qc", "Ah", "2c", "3d", "4h", "5s", "6c"]
            .iter()
            .map(|s| c(s))
            .collect(),
    )
    .unwrap();
    let mut hand = HandState::new(cfg, deck).unwrap();
    let abs = PreflopClassAbstraction;
    // Button (seat 0) holds As Ah.
    let k = InfoKey::from_observation(&hand.observation(Seat::Zero), &abs).unwrap();
    assert_eq!(k.to_string(), "BTN#0 Pre:");
    assert_eq!(k.to_bytes(), vec![1, 0, 0, 0, 0, 0]);
    // Not to act / wrong seat.
    assert_eq!(
        InfoKey::from_observation(&hand.observation(Seat::One), &abs),
        Err(KeyError::NotActing)
    );
    hand.apply(ActionKind::Call).unwrap();
    let k = InfoKey::from_observation(&hand.observation(Seat::One), &abs).unwrap();
    assert_eq!(k.to_string(), format!("BB#{} Pre:c", cls("KQo").index()));
    hand.apply(ActionKind::Check).unwrap();
    // Flop: preflop-only abstraction refuses, it never invents a bucket.
    assert_eq!(abs.num_buckets(Street::Flop), 0);
    assert_eq!(
        InfoKey::from_observation(&hand.observation(Seat::One), &abs),
        Err(KeyError::Abstraction(AbstractionError::UnsupportedStreet(
            Street::Flop
        )))
    );
    let k = InfoKey::from_observation(&hand.observation(Seat::One), &ExactCards).unwrap();
    assert_eq!(k.history().to_string(), "Pre:c-k|Flop:");
    hand.apply(ActionKind::Pot).unwrap();
    hand.apply(ActionKind::Fold).unwrap();
    assert_eq!(
        InfoKey::from_observation(&hand.observation(Seat::Zero), &abs),
        Err(KeyError::Terminal)
    );
}

// ------------------------------------------------------------------ hidden information

/// For random hands, re-randomize the opponent's hole cards and every board
/// card not yet visible to the acting seat, replay the same public actions,
/// and require identical keys for that seat at every decision.
#[test]
fn preflop_hidden_information_swap_keeps_keys_identical() {
    let mut rng = PokerbotRng::seed_from_u64(77);
    let mut compared = 0usize;
    let mut opp_changed = 0usize;
    for i in 0..400u64 {
        let cfg = config(
            StackDepth::ALL[(i % 3) as usize],
            Seat::ALL[(i % 2) as usize],
        );
        let deck_a = Deck::from_seed(1_000 + i);
        let order_a = deck_a.order().to_vec();
        let (kinds, states_a) = play(cfg, deck_a, &mut rng, i % 3 == 0);
        for seat in Seat::ALL {
            // Deck positions: BB hole 0,2; button hole 1,3; board 4..9.
            let own: [usize; 2] = if seat == cfg.button { [1, 3] } else { [0, 2] };
            for visible in [0usize, 3, 4, 5] {
                let fixed: Vec<usize> = own.iter().copied().chain(4..4 + visible).collect();
                let free: Vec<usize> = (0..52).filter(|p| !fixed.contains(p)).collect();
                let mut order_b = order_a.clone();
                // Fisher-Yates over the free positions only.
                for j in (1..free.len()).rev() {
                    let r = rng.gen_range(j as u64 + 1) as usize;
                    order_b.swap(free[j], free[r]);
                }
                let opp: [usize; 2] = if own == [1, 3] { [0, 2] } else { [1, 3] };
                if opp.iter().any(|&p| order_a[p] != order_b[p]) {
                    opp_changed += 1;
                }
                let states_b = replay(cfg, Deck::from_order(order_b).unwrap(), &kinds);
                assert_eq!(states_a.len(), states_b.len());
                for (a, b) in states_a.iter().zip(&states_b) {
                    if a.to_act() != Some(seat) || a.board().len() > visible {
                        continue;
                    }
                    for abs in [
                        &PreflopClassAbstraction as &dyn CardAbstraction,
                        &ExactCards,
                    ] {
                        match (key_for(a, abs), key_for(b, abs)) {
                            (Some(ka), Some(kb)) => {
                                assert_eq!(ka, kb, "key leaked hidden info (hand {i})");
                                assert_eq!(ka.1.hash64(), kb.1.hash64());
                                compared += 1;
                            }
                            (None, None) => {}
                            _ => panic!("key availability differed"),
                        }
                    }
                }
            }
        }
    }
    eprintln!("compared {compared} key pairs; opponent cards changed in {opp_changed} decks");
    assert!(compared > 2_000, "too few comparisons: {compared}");
    assert!(
        opp_changed > 3_000,
        "swap rarely changed opponent cards: {opp_changed}"
    );
}

/// Sensitivity check: the swap test would catch a leak, because changing the
/// actor's *own* cards does change the ExactCards bucket.
#[test]
fn preflop_own_cards_do_change_key() {
    let cfg = config(StackDepth::Bb100, Seat::Zero);
    let mut order: Vec<Card> = Deck::from_seed(5).order().to_vec();
    let a = HandState::new(cfg, Deck::from_order(order.clone()).unwrap()).unwrap();
    order.swap(1, 20); // button's first hole card
    let b = HandState::new(cfg, Deck::from_order(order).unwrap()).unwrap();
    let ka = key_for(&a, &ExactCards).unwrap();
    let kb = key_for(&b, &ExactCards).unwrap();
    assert_ne!(ka, kb);
}
