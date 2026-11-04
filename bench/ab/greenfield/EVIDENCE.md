# Blank-slate A/B: by model and arm

A = no hexa, the model designs from a behaviour-only spec. B = the same spec through `hexa scaffold`.
Defects are bench/probes, hidden from both arms.

## claude-haiku-5-5

| | A | B |
|---|---:|---:|
| runs / void | 5 / 0 | 5 / 0 |
| **build: gate passes** | 100% | 100% |
| **build: hidden defects, mean** | 1.40 | 1 |
| build: gate passes and 0 defects | 0% | 20% |
| change: gate passes on SQLite | 100% | 100% |
| change: hidden defects, mean | 1.60 | 0.80 |
| build $, median | 0.01 | 0.69 |
| change $, median | 0.01 | 0.14 |
| total $ | 0.13 | 4.82 |
| build minutes, median | 1.0 | 17.7 |
| source files, median | 4 | 23 |
| source lines, median | 169 | 815 |
| test lines, median | 0 | 1019 |
| builds that wrote tests | 0% | 100% |
| CCN mean / max, median | 2.8 / 6 | 2.1 / 9 |
| change: source files touched, median | 1 | 3 |
| probe p1-wide-codepoint-url (build / change) | 40% / 40% | 0% / 0% |
| probe p2-chunked-body-cap (build / change) | 100% / 100% | 0% / 0% |
| probe p4-malformed-escape (build / change) | 0% / 0% | 0% / 0% |
| probe p5-partial-corruption (build / change) | 0% / 0% | 40% / 0% |
| probe p6-empty-store-dir (build / change) | 0% / 0% | 0% / 0% |
| probe p7-stale-store-handle (build / change) | 0% / 20% | 60% / 80% |

## claude-opus-5-5

| | A | B |
|---|---:|---:|
| runs / void | 5 / 0 | 5 / 0 |
| **build: gate passes** | 100% | 100% |
| **build: hidden defects, mean** | 2.20 | 0.20 |
| build: gate passes and 0 defects | 0% | 80% |
| change: gate passes on SQLite | 80% | 100% |
| change: hidden defects, mean | 3 | 0.80 |
| build $, median | 0.28 | 4.58 |
| change $, median | 0.32 | 0.92 |
| total $ | 2.95 | 27.82 |
| build minutes, median | 0.8 | 15.4 |
| source files, median | 2 | 21 |
| source lines, median | 127 | 579 |
| test lines, median | 0 | 689 |
| builds that wrote tests | 0% | 100% |
| CCN mean / max, median | 3.3 / 6 | 2.2 / 11 |
| change: source files touched, median | 1 | 4 |
| probe p1-wide-codepoint-url (build / change) | 100% / 100% | 0% / 0% |
| probe p2-chunked-body-cap (build / change) | 100% / 100% | 0% / 0% |
| probe p4-malformed-escape (build / change) | 0% / 0% | 0% / 0% |
| probe p5-partial-corruption (build / change) | 20% / 0% | 0% / 0% |
| probe p6-empty-store-dir (build / change) | 0% / 0% | 0% / 0% |
| probe p7-stale-store-handle (build / change) | 0% / 100% | 20% / 80% |

## claude-sonnet-5-5

| | A | B |
|---|---:|---:|
| runs / void | 5 / 0 | 5 / 0 |
| **build: gate passes** | 100% | 100% |
| **build: hidden defects, mean** | 2 | 0.40 |
| build: gate passes and 0 defects | 0% | 60% |
| change: gate passes on SQLite | 100% | 100% |
| change: hidden defects, mean | 2.20 | 0.80 |
| build $, median | 0.12 | 1.68 |
| change $, median | 0.11 | 0.32 |
| total $ | 1.17 | 10.07 |
| build minutes, median | 0.5 | 9.2 |
| source files, median | 2 | 19 |
| source lines, median | 105 | 481 |
| test lines, median | 0 | 518 |
| builds that wrote tests | 0% | 100% |
| CCN mean / max, median | 3.0 / 6 | 2.2 / 12 |
| change: source files touched, median | 1 | 3 |
| probe p1-wide-codepoint-url (build / change) | 100% / 100% | 0% / 0% |
| probe p2-chunked-body-cap (build / change) | 100% / 100% | 0% / 0% |
| probe p4-malformed-escape (build / change) | 0% / 0% | 0% / 0% |
| probe p5-partial-corruption (build / change) | 0% / 0% | 0% / 0% |
| probe p6-empty-store-dir (build / change) | 0% / 0% | 0% / 0% |
| probe p7-stale-store-handle (build / change) | 0% / 20% | 40% / 80% |

## all models

| | A | B |
|---|---:|---:|
| runs / void | 15 / 0 | 15 / 0 |
| **build: gate passes** | 100% | 100% |
| **build: hidden defects, mean** | 1.87 | 0.53 |
| build: gate passes and 0 defects | 0% | 53% |
| change: gate passes on SQLite | 93% | 100% |
| change: hidden defects, mean | 2.27 | 0.80 |
| build $, median | 0.12 | 1.68 |
| change $, median | 0.11 | 0.33 |
| total $ | 4.25 | 42.71 |
| build minutes, median | 0.8 | 14.9 |
| source files, median | 2 | 21 |
| source lines, median | 127 | 582 |
| test lines, median | 0 | 689 |
| builds that wrote tests | 0% | 100% |
| CCN mean / max, median | 3.0 / 6 | 2.2 / 11 |
| change: source files touched, median | 1 | 3 |
| probe p1-wide-codepoint-url (build / change) | 80% / 80% | 0% / 0% |
| probe p2-chunked-body-cap (build / change) | 100% / 100% | 0% / 0% |
| probe p4-malformed-escape (build / change) | 0% / 0% | 0% / 0% |
| probe p5-partial-corruption (build / change) | 7% / 0% | 13% / 0% |
| probe p6-empty-store-dir (build / change) | 0% / 0% | 0% / 0% |
| probe p7-stale-store-handle (build / change) | 0% / 47% | 40% / 80% |
