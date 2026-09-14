//! The seed promise: the same seed repeats, and different seeds diverge.

mod common;

use std::collections::HashSet;
use std::process::Command;

use common::play_seed;

/// One thousand seeds, each played twice in one process. Byte for byte equal.
#[test]
fn same_seed_same_game() {
    for seed in 1..=1000_u64 {
        let first = play_seed(seed);
        let second = play_seed(seed);
        assert_eq!(first, second, "seed {seed} played two different games");
    }
}

/// One thousand seeds, one thousand different games. Zero collisions. This is
/// an exact number, not "a high number".
#[test]
fn distinct_seeds_distinct_games() {
    let mut seen: HashSet<String> = HashSet::new();
    for seed in 1..=1000_u64 {
        seen.insert(play_seed(seed));
    }
    assert_eq!(seen.len(), 1000, "two seeds played the same game");
}

/// Neighbouring seeds must differ too. SplitMix64 mixes the seed itself, so
/// seed 41 and seed 42 part company on the very first number.
#[test]
fn neighbouring_seeds_differ() {
    for seed in 0..50_u64 {
        assert_ne!(
            play_seed(seed),
            play_seed(seed + 1),
            "seed {seed} and seed {} played the same game",
            seed + 1
        );
    }
}

/// One hundred runs of the real binary on seed 7. The bytes must be identical
/// every time, across processes, not only inside one.
#[test]
fn binary_is_repeatable() {
    let binary = env!("CARGO_BIN_EXE_connect-four");
    let first = Command::new(binary)
        .args(["--demo", "--seed", "7"])
        .output()
        .expect("the binary runs")
        .stdout;
    assert!(!first.is_empty(), "seed 7 printed nothing");

    for attempt in 1..100 {
        let again = Command::new(binary)
            .args(["--demo", "--seed", "7"])
            .output()
            .expect("the binary runs")
            .stdout;
        assert_eq!(again, first, "run {attempt} of seed 7 came out different");
    }
}
