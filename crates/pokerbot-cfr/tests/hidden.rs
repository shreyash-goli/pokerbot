//! Hidden-information tests for the MCCFR trainer.
//! Tests are prefixed `hidden_` so `cargo test -p pokerbot-cfr hidden` selects them.

use pokerbot_cfr::abstraction::key::{CardAbstraction, InfoKey};
use pokerbot_cfr::traverse::{Trainer, TrainerConfig};
use pokerbot_core::action::Seat;
use pokerbot_core::card::Card;
use pokerbot_core::deck::Deck;
use pokerbot_core::rng::PokerbotRng;
use pokerbot_core::rules::{ActionKind, Position};
use pokerbot_core::state::{HandConfig, HandState};

/// Deck positions: BB hole 0,2; button hole 1,3; board 4..9. Button = seat Zero.
fn own_positions(seat: Seat) -> [usize; 2] {
    if seat == Seat::Zero { [1, 3] } else { [0, 2] }
}

/// Copy of `order` with the opponent's hole cards, the board beyond the first
/// `visible` cards and the rest of the deck re-shuffled; `seat`'s own hole
/// cards and the visible board are kept.
fn swap_hidden(order: &[Card], seat: Seat, visible: usize, rng: &mut PokerbotRng) -> Vec<Card> {
    let own = own_positions(seat);
    let fixed: Vec<usize> = own.iter().copied().chain(4..4 + visible).collect();
    let free: Vec<usize> = (0..order.len()).filter(|p| !fixed.contains(p)).collect();
    let mut out = order.to_vec();
    for j in (1..free.len()).rev() {
        let r = rng.gen_range(j as u64 + 1) as usize;
        out.swap(free[j], free[r]);
    }
    out
}

/// Every fast-path key built during training equals
/// `InfoKey::from_observation(&state.observation(actor), ..)`.
#[test]
fn hidden_fast_path_keys_equal_observation_keys() {
    let mut cfg = TrainerConfig::tiny(5);
    cfg.verify_keys = true;
    let mut t = Trainer::new(cfg).unwrap();
    t.run_iterations(400);
    let s = t.stats();
    assert_eq!(s.keys_verified, s.nodes, "every node verified");
    assert!(s.keys_verified > 10_000, "only {} nodes", s.keys_verified);
    eprintln!("verified {} fast-path keys", s.keys_verified);
}

/// The non-incremental key helper (`node_key_bytes`) also matches
/// `from_observation`, and is unchanged when the opponent's cards or undealt
/// board cards are swapped, at every decision of random hands.
#[test]
fn hidden_node_keys_ignore_opponent_cards_and_future_board() {
    let t = Trainer::new(TrainerConfig::tiny(1)).unwrap();
    let cfg = HandConfig {
        rules: t.config().rules,
        button: Seat::Zero,
    };
    let mut rng = PokerbotRng::seed_from_u64(9);
    let mut compared = 0;
    for i in 0..120u64 {
        let order = Deck::from_seed(500 + i).order().to_vec();
        // Random public action sequence (legality never depends on cards).
        let mut hand = HandState::new(cfg, Deck::from_order(order.clone()).unwrap()).unwrap();
        let mut kinds = Vec::new();
        while !hand.is_terminal() {
            let legal = hand.legal_actions();
            let k = legal[rng.gen_range(legal.len() as u64) as usize].kind;
            hand.apply(k).unwrap();
            kinds.push(k);
        }
        for seat in Seat::ALL {
            for visible in [0usize, 3, 4, 5] {
                let order_b = swap_hidden(&order, seat, visible, &mut rng);
                let mut a = HandState::new(cfg, Deck::from_order(order.clone()).unwrap()).unwrap();
                let mut b = HandState::new(cfg, Deck::from_order(order_b).unwrap()).unwrap();
                for &k in &kinds {
                    if a.to_act() == Some(seat) && a.board().len() <= visible {
                        let ka = t.node_key_bytes(&a).unwrap();
                        let kb = t.node_key_bytes(&b).unwrap();
                        assert_eq!(ka, kb, "hand {i}: key changed with hidden cards");
                        let obs = InfoKey::from_observation(&a.observation(seat), t.abstraction())
                            .unwrap()
                            .to_bytes();
                        assert_eq!(ka, obs);
                        compared += 1;
                    }
                    a.apply(k).unwrap();
                    b.apply(k).unwrap();
                }
            }
        }
    }
    eprintln!("compared {compared} decision keys");
    assert!(compared > 1_000);
}

/// Training-level swap: on a fresh trainer every opponent strategy is
/// uniform, so with the same RNG the sampled path does not depend on the
/// opponent's key. A traversal on two deals that differ only in the
/// opponent's hole cards must produce the same traverser information sets.
#[test]
fn hidden_training_swap_keeps_traverser_keys() {
    let mut rng = PokerbotRng::seed_from_u64(31);
    let mut checked = 0;
    for i in 0..60u64 {
        let order = Deck::from_seed(9_000 + i).order().to_vec();
        for traverser in Seat::ALL {
            let opp = own_positions(traverser.other());
            let mut order_b = order.clone();
            // Swap the opponent's hole cards with two cards outside the
            // first nine (never dealt), leaving everything else fixed.
            let x = 9 + rng.gen_range(43) as usize;
            let mut y = 9 + rng.gen_range(43) as usize;
            if y == x {
                y = if x == 51 { 9 } else { x + 1 };
            }
            order_b.swap(opp[0], x);
            order_b.swap(opp[1], y);
            let pos = if traverser == Seat::Zero {
                Position::Button
            } else {
                Position::BigBlind
            };
            let keys = |o: Vec<Card>| {
                let mut t = Trainer::new(TrainerConfig::tiny(77)).unwrap();
                t.traverse_deck(Deck::from_order(o).unwrap(), traverser);
                t.table()
                    .iter()
                    .filter(|(k, _)| InfoKey::from_bytes(k).unwrap().position() == pos)
                    .map(|(k, e)| (k.to_vec(), e.mask))
                    .collect::<Vec<_>>()
            };
            let (ka, kb) = (keys(order.clone()), keys(order_b));
            assert!(!ka.is_empty());
            assert_eq!(
                ka, kb,
                "deal {i}: traverser keys changed with opponent cards"
            );
            checked += ka.len();
        }
    }
    eprintln!("checked {checked} traverser information sets");
}

/// Every stored key decodes as an `InfoKey` whose bucket is in range for its
/// street. Keys carry only street, own position, own bucket and public
/// action classes (format v1, 6 header bytes + history), with no card or
/// opponent field.
#[test]
fn hidden_table_keys_contain_only_observation_fields() {
    let mut t = Trainer::new(TrainerConfig::tiny(2)).unwrap();
    t.run_iterations(200);
    for (k, _) in t.table().iter() {
        let key = InfoKey::from_bytes(k).unwrap();
        assert_eq!(k.len(), 6 + key.history().len());
        assert!(key.bucket() < t.abstraction().num_buckets(key.street()));
        for acts in key.history().streets() {
            assert!(acts.iter().all(|a| ActionKind::ALL.contains(a)));
        }
    }
}
