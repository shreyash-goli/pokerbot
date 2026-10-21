# Pokerbot strategy artifact format (`.pokerbot`), version 1

Status: specification for M-12. Implemented by
`crates/pokerbot-cfr/src/artifact.rs`, tested by `crates/pokerbot-cfr/tests/artifact.rs`.
This is an original format, written independently. No other
poker project's format or code was used.

A `.pokerbot` file stores the **average strategy** of an external-sampling MCCFR
table (M-10/M-11), quantized per legal action. It also stores the metadata
needed to refuse use with the wrong game. It does not store regrets; resumable
training uses the separate checkpoint format (`checkpoint.rs`, magic
`PKBTCKPT`).

A policy loaded from an artifact approximates an equilibrium of the
*abstract* game (169 preflop classes, equity buckets, six actions) for one
stack depth. It is **not** GTO and makes no strength claim; the `label` field
says what the run was for (e.g. `smoke: trained for plumbing, not competition`).

## Byte layout

All integers are little-endian. Strings are UTF-8 with a length prefix and no
terminator.

| # | Field | Type | Notes |
|---|---|---|---|
| 1 | magic | 8 bytes | `PKBTSTRT` |
| 2 | format version | `u16` | `1` |
| 3 | rules-spec length | `u8` | |
| 4 | rules spec | bytes | `GAME_RULES v1` (docs/GAME_RULES.md) |
| 5 | small blind | `u32` | |
| 6 | big blind | `u32` | |
| 7 | starting stack | `u32` | chips; together with the blinds this fixes the depth |
| 8 | abstraction config | 28 bytes | `AbstractionConfig::to_bytes` (see below) |
| 9 | abstraction fingerprint | `u64` | FNV-1a 64 of field 8; must match |
| 10 | trainer flags | `u32` | bit0 RM+, bit1 linear averaging, bit2 linear regrets; other bits must be 0 |
| 11 | trainer seed | `u64` | |
| 12 | iterations | `u64` | completed training iterations |
| 13 | trainer id length | `u16` | ≤ 4096 |
| 14 | trainer id | bytes | e.g. `pokerbot-cfr 0.1.0 es-mccfr` |
| 15 | label length | `u16` | ≤ 4096 |
| 16 | label | bytes | free text |
| 17 | entry count `n` | `u32` | |
| 18 | `n` entries | | see below, sorted by key bytes |
| 19 | checksum | `u64` | FNV-1a 64 over every byte of fields 1–18 |

Each entry:

| Field | Type | Notes |
|---|---|---|
| key length | `u8` | `6 + history length` (≤ 141) |
| key | bytes | `InfoKey` v1 bytes (`abstraction/key.rs`): version, street, position, bucket `u16`, history length, history |
| legal mask | `u8` | bit `i` = action slot `i` legal; slots `Fold 0, Check 1, Call 2, HalfPot 3, Pot 4, AllIn 5` |
| probabilities | `u16` × popcount(mask) | legal slots only, in slot order |

`AbstractionConfig` bytes (28): `MKEQ`, version `u16`, flop/turn/river
bucket counts `u16`×3, flop samples `u32`, turn samples `u32` (0 = exact),
seed `u64`.

No timestamp or other environment-dependent value is stored, so the same table
and metadata always produce byte-identical files with the same checksum.

## Quantization

Each entry's probabilities are integers that sum to **exactly 65535**
(probability = value / 65535). They come from the normalized average strategy
`p` by largest-remainder rounding:

1. `x[i] = p[i] / Σ p · 65535` over legal slots (uniform if `Σ p = 0`).
2. `q[i] = floor(x[i])`.
3. The `65535 − Σ q` leftover units go one each to legal slots in order of
   decreasing fractional part `x[i] − q[i]`; ties go to the lower slot index.

Each stored probability is within 1/65535 of the trained value. Information
sets with no accumulated average-strategy mass (visited only as the
traverser) are omitted; a lookup for them returns "no entry", and the caller
chooses the fallback.

## Validation (reader)

The reader is pure (no file system) and runs the same checks natively and in
wasm. It rejects the input, with a specific error, if any of these fail:

1. Magic is `PKBTSTRT`.
2. Format version is supported (currently only `1`).
3. Checksum matches (corruption detection only: FNV-1a is **not** a security
   measure, and anyone can recompute it).
4. Every field is fully present (no truncation), and there are no bytes
   between the last entry and the checksum.
5. The rules spec is a supported string (`GAME_RULES v1`).
6. The rules config passes `RulesConfig::validate`. The abstraction config
   decodes and validates, and its fingerprint matches.
7. The flags contain only known bits, and both text fields are UTF-8 of at
   most 4096 bytes.
8. Keys are strictly increasing in byte order (sorted, no duplicates).
9. Every key decodes as an `InfoKey`, and its bucket is within the
   abstraction's bucket count for its street (169 preflop).
10. Every mask is non-empty, uses only the six slots, and **equals the legal
    action set** obtained by replaying the key's public history under the
    artifact's rules. A mask that grants an illegal action, a history that
    cannot occur, or a position that is not the player to act is rejected.
11. Illegal slots carry zero probability, and the probabilities sum to 65535.

`Policy::load(bytes, rules, abstraction)` also requires the caller's
`RulesConfig` (blinds and starting stack) and `AbstractionConfig` to equal
the artifact's. Loading a 20 BB artifact into a 100 BB game fails with
`RulesMismatch`, and a different abstraction fails with `AbstractionMismatch`.

## Versioning policy

- Any change to the byte layout, the quantization rule or the meaning of any
  field bumps the **format version**. Readers reject versions they do not
  know; there is no best-effort parsing.
- A change to the betting rules or action semantics bumps the **rules spec**
  string (`GAME_RULES v2`, …) together with docs/GAME_RULES.md. Old artifacts
  are then rejected, not reinterpreted.
- A change to the information-set key layout bumps `KEY_FORMAT_VERSION`
  (abstraction/key.rs) and the artifact format version.
- A change to card bucketing bumps `POSTFLOP_ABSTRACTION_VERSION`. That
  changes the abstraction bytes and fingerprint, so `Policy::load` rejects
  the mismatch.
- Trained artifacts are generated outputs. They are not committed except for
  tiny, intentionally versioned test fixtures.

## Size (measured)

Tiny 20 BB config (3/3/3 buckets), 10,000 iterations: 21,796 information sets
in the table, of which 17,516 are stored. The file is 370,286 bytes (≈362 KiB),
and load plus full validation takes about 6 ms in a release build.
