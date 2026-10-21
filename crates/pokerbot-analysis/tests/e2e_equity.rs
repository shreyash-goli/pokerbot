//! M-24 cross-check: the coach's exact equity (`pokerbot_analysis::equity`) and
//! the trainer's exact equity (`pokerbot_cfr::equity::exact_equity`) are two
//! independent implementations; on seeded river and turn spots they must
//! agree exactly (same win/tie point tally over the same trial count).

use pokerbot_analysis::equity::{Method, vs_random};
use pokerbot_cfr::equity::exact_equity;
use pokerbot_core::card::Card;
use pokerbot_core::deck::Deck;
use pokerbot_core::rng::PokerbotRng;

fn check(rng: &mut PokerbotRng, board_len: usize, n: usize) {
    for _ in 0..n {
        let d = Deck::shuffled(rng).deal_n(2 + board_len).unwrap();
        let hole: [Card; 2] = [d[0], d[1]];
        let board = &d[2..];
        let coach = vs_random(hole, board).expect("valid spot");
        assert_eq!(coach.method, Method::Exact);
        let trainer = exact_equity(hole, board);
        assert_eq!(
            (coach.points, coach.trials),
            (trainer.points, trainer.trials),
            "hole {hole:?} board {board:?}"
        );
        assert_eq!(coach.value().unwrap(), trainer.value());
    }
}

#[test]
fn e2e_equity_river_exact_matches_trainer() {
    // 990 opponent hands per river spot.
    check(&mut PokerbotRng::seed_from_u64(0xE0_5175), 5, 300);
}

#[test]
fn e2e_equity_turn_exact_matches_trainer() {
    // 44 rivers x 946 opponent hands per turn spot.
    check(&mut PokerbotRng::seed_from_u64(0xE0_7042), 4, 120);
}
