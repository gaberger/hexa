# hexa A/B evidence, by family and model

# Change existing code (the hexa fixtures)

Tasks: declared-size-cap, header-safe-url, highest-version, runs-feed-filter, store-identity.

## claude-haiku-5-5

| metric | A | B | C |
|---|---:|---:|---:|
| runs launched / void | 25 / 0 | 25 / 0 | 25 / 0 |
| **accepted-correct** | 100% | 100% | 96% |
| oracle pass | 100% | 100% | 96% |
| own gate pass | 100% | 100% | 96% |
| false-green (correctness) | 0% | 0% | 0% |
| false-green (shape) | 0% | 0% | 0% |
| no change delivered | 0% | 0% | 4% |
| timeouts | 0% | 0% | 0% |
| violations, median | — | — | — |
| $ per run, median | 0.006 | 0.005 | 0.006 |
| tokens per run, median | 196997 | 193508 | 249952 |
| wall s per run, median | 20 | 19 | 21 |
| $ per accepted-correct | 0.006 | 0.005 | 0.007 |
| tokens per accepted-correct | 219584 | 201660 | 263377 |
| src NLOC, median | 13 | 13 | 14 |
| test NLOC, median | 47 | 26 | 44 |
| lines changed, median | 69 | 45 | 57 |
| CCN mean, median | 4.0 | 4.0 | 3.0 |
| CCN max, median | 4 | 4 | 4 |
| longest function, median | 6 | 6 | 6 |

- **B−A** accepted-correct: +0.0 points, 95% CI [+0.0, +0.0], over 5 tasks
- **C−A** accepted-correct: -4.0 points, 95% CI [-16.0, +0.0], over 5 tasks

## claude-opus-5-5

| metric | A | B | C |
|---|---:|---:|---:|
| runs launched / void | 25 / 0 | 25 / 0 | 25 / 0 |
| **accepted-correct** | 100% | 100% | 100% |
| oracle pass | 100% | 100% | 100% |
| own gate pass | 100% | 100% | 100% |
| false-green (correctness) | 0% | 0% | 0% |
| false-green (shape) | 0% | 0% | 0% |
| no change delivered | 0% | 0% | 0% |
| timeouts | 0% | 0% | 0% |
| violations, median | — | — | — |
| $ per run, median | 0.176 | 0.161 | 0.179 |
| tokens per run, median | 152375 | 152266 | 201669 |
| wall s per run, median | 32 | 27 | 34 |
| $ per accepted-correct | 0.179 | 0.167 | 0.189 |
| tokens per accepted-correct | 185794 | 177907 | 201272 |
| src NLOC, median | 13 | 13 | 14 |
| test NLOC, median | 42 | 26 | 47 |
| lines changed, median | 67 | 44 | 73 |
| CCN mean, median | 4.0 | 4.0 | 4.0 |
| CCN max, median | 4 | 4 | 4 |
| longest function, median | 6 | 6 | 6 |

- **B−A** accepted-correct: +0.0 points, 95% CI [+0.0, +0.0], over 5 tasks
- **C−A** accepted-correct: +0.0 points, 95% CI [+0.0, +0.0], over 5 tasks

## claude-sonnet-5-5

| metric | A | B | C |
|---|---:|---:|---:|
| runs launched / void | 25 / 0 | 25 / 0 | 25 / 0 |
| **accepted-correct** | 100% | 100% | 100% |
| oracle pass | 100% | 100% | 100% |
| own gate pass | 100% | 100% | 100% |
| false-green (correctness) | 0% | 0% | 0% |
| false-green (shape) | 0% | 0% | 0% |
| no change delivered | 0% | 0% | 0% |
| timeouts | 0% | 0% | 0% |
| violations, median | — | — | — |
| $ per run, median | 0.073 | 0.061 | 0.077 |
| tokens per run, median | 148675 | 111112 | 160926 |
| wall s per run, median | 21 | 18 | 24 |
| $ per accepted-correct | 0.079 | 0.066 | 0.080 |
| tokens per accepted-correct | 175560 | 127872 | 179548 |
| src NLOC, median | 12 | 13 | 12 |
| test NLOC, median | 20 | 20 | 21 |
| lines changed, median | 31 | 26 | 32 |
| CCN mean, median | 4.0 | 4.0 | 4.0 |
| CCN max, median | 4 | 4 | 4 |
| longest function, median | 6 | 6 | 6 |

- **B−A** accepted-correct: +0.0 points, 95% CI [+0.0, +0.0], over 5 tasks
- **C−A** accepted-correct: +0.0 points, 95% CI [+0.0, +0.0], over 5 tasks

# Shape traps (hexa's own examples)

Tasks: linkstore-cache, sokoban-untangle.

## claude-haiku-5-5

| metric | A | B | C |
|---|---:|---:|---:|
| runs launched / void | 10 / 0 | 10 / 0 | 10 / 0 |
| **accepted-correct** | 100% | 100% | 100% |
| oracle pass | 100% | 100% | 100% |
| own gate pass | 100% | 100% | 100% |
| false-green (correctness) | 0% | 0% | 0% |
| false-green (shape) | 0% | 0% | 0% |
| no change delivered | 0% | 0% | 0% |
| timeouts | 0% | 0% | 0% |
| violations, median | 0 | 0 | 0 |
| $ per run, median | 0.025 | 0.019 | 0.024 |
| tokens per run, median | 923945 | 683678 | 893070 |
| wall s per run, median | 88 | 73 | 83 |
| $ per accepted-correct | 0.025 | 0.020 | 0.025 |
| tokens per accepted-correct | 908441 | 713602 | 899290 |
| src NLOC, median | 632 | 626 | 626 |
| test NLOC, median | 400 | 355 | 407 |
| lines changed, median | 211 | 92 | 204 |
| CCN mean, median | 2.3 | 2.3 | 2.3 |
| CCN max, median | 9 | 9 | 9 |
| longest function, median | 33 | 33 | 33 |

- **B−A** accepted-correct: +0.0 points, 95% CI [+0.0, +0.0], over 2 tasks
- **C−A** accepted-correct: +0.0 points, 95% CI [+0.0, +0.0], over 2 tasks

## claude-opus-5-5

| metric | A | B | C |
|---|---:|---:|---:|
| runs launched / void | 10 / 0 | 10 / 0 | 10 / 0 |
| **accepted-correct** | 100% | 100% | 100% |
| oracle pass | 100% | 100% | 100% |
| own gate pass | 100% | 100% | 100% |
| false-green (correctness) | 0% | 0% | 0% |
| false-green (shape) | 0% | 0% | 0% |
| no change delivered | 0% | 0% | 0% |
| timeouts | 0% | 0% | 0% |
| violations, median | 0 | 0 | 0 |
| $ per run, median | 0.608 | 0.427 | 0.639 |
| tokens per run, median | 721724 | 457549 | 756230 |
| wall s per run, median | 98 | 74 | 109 |
| $ per accepted-correct | 0.623 | 0.466 | 0.648 |
| tokens per accepted-correct | 696337 | 483936 | 743221 |
| src NLOC, median | 632 | 630 | 630 |
| test NLOC, median | 355 | 355 | 398 |
| lines changed, median | 176 | 91 | 234 |
| CCN mean, median | 2.3 | 2.3 | 2.3 |
| CCN max, median | 9 | 9 | 9 |
| longest function, median | 33 | 33 | 33 |

- **B−A** accepted-correct: +0.0 points, 95% CI [+0.0, +0.0], over 2 tasks
- **C−A** accepted-correct: +0.0 points, 95% CI [+0.0, +0.0], over 2 tasks

## claude-sonnet-5-5

| metric | A | B | C |
|---|---:|---:|---:|
| runs launched / void | 10 / 0 | 10 / 0 | 10 / 0 |
| **accepted-correct** | 100% | 90% | 100% |
| oracle pass | 100% | 100% | 100% |
| own gate pass | 100% | 100% | 100% |
| false-green (correctness) | 0% | 0% | 0% |
| false-green (shape) | 0% | 0% | 0% |
| no change delivered | 0% | 10% | 0% |
| timeouts | 0% | 0% | 0% |
| violations, median | 0 | 0 | 0 |
| $ per run, median | 0.220 | 0.164 | 0.226 |
| tokens per run, median | 482032 | 371150 | 451222 |
| wall s per run, median | 58 | 69 | 64 |
| $ per accepted-correct | 0.222 | 0.221 | 0.223 |
| tokens per accepted-correct | 474177 | 487419 | 474186 |
| src NLOC, median | 628 | 623 | 622 |
| test NLOC, median | 355 | 355 | 355 |
| lines changed, median | 78 | 57 | 75 |
| CCN mean, median | 2.3 | 2.3 | 2.3 |
| CCN max, median | 9 | 10 | 9 |
| longest function, median | 33 | 33 | 33 |

- **B−A** accepted-correct: -10.0 points, 95% CI [-40.0, +0.0], over 2 tasks
- **C−A** accepted-correct: +0.0 points, 95% CI [+0.0, +0.0], over 2 tasks

# Refactoring entangled dependencies

Tasks: linkstore-untangle.

## claude-haiku-5-5

| metric | A | B | C |
|---|---:|---:|---:|
| runs launched / void | 5 / 0 | 5 / 0 | 5 / 0 |
| **accepted-correct** | 60% | 100% | 60% |
| oracle pass | 100% | 100% | 100% |
| own gate pass | 100% | 100% | 100% |
| false-green (correctness) | 0% | 0% | 0% |
| false-green (shape) | 40% | 0% | 40% |
| no change delivered | 0% | 0% | 0% |
| timeouts | 0% | 0% | 0% |
| violations, median | 0 | 0 | 0 |
| $ per run, median | 0.041 | 0.044 | 0.045 |
| tokens per run, median | 1658638 | 1043578 | 1622480 |
| wall s per run, median | 117 | 115 | 123 |
| $ per accepted-correct | 0.081 | 0.049 | 0.084 |
| tokens per accepted-correct | 2792103 | 1115244 | 2990440 |
| src NLOC, median | 829 | 829 | 836 |
| test NLOC, median | 510 | 510 | 510 |
| lines changed, median | 136 | 155 | 153 |
| CCN mean, median | 2.1 | 2.1 | 2.1 |
| CCN max, median | 8 | 8 | 8 |
| longest function, median | 33 | 33 | 33 |

- **B−A** accepted-correct: +40.0 points, 95% CI [+0.0, +80.0], over 1 tasks
- **C−A** accepted-correct: +0.0 points, 95% CI [-60.0, +60.0], over 1 tasks

## claude-opus-5-5

| metric | A | B | C |
|---|---:|---:|---:|
| runs launched / void | 5 / 0 | 5 / 0 | 5 / 0 |
| **accepted-correct** | 100% | 100% | 100% |
| oracle pass | 100% | 100% | 100% |
| own gate pass | 100% | 100% | 100% |
| false-green (correctness) | 0% | 0% | 0% |
| false-green (shape) | 0% | 0% | 0% |
| no change delivered | 0% | 0% | 0% |
| timeouts | 0% | 0% | 0% |
| violations, median | 0 | 0 | 0 |
| $ per run, median | 0.735 | 0.970 | 0.912 |
| tokens per run, median | 801493 | 1135667 | 1039880 |
| wall s per run, median | 132 | 136 | 126 |
| $ per accepted-correct | 0.802 | 0.971 | 0.916 |
| tokens per accepted-correct | 881050 | 1159466 | 1040276 |
| src NLOC, median | 828 | 827 | 828 |
| test NLOC, median | 510 | 510 | 510 |
| lines changed, median | 137 | 148 | 131 |
| CCN mean, median | 2.1 | 2.1 | 2.1 |
| CCN max, median | 8 | 8 | 8 |
| longest function, median | 33 | 33 | 33 |

- **B−A** accepted-correct: +0.0 points, 95% CI [+0.0, +0.0], over 1 tasks
- **C−A** accepted-correct: +0.0 points, 95% CI [+0.0, +0.0], over 1 tasks

## claude-sonnet-5-5

| metric | A | B | C |
|---|---:|---:|---:|
| runs launched / void | 5 / 0 | 5 / 0 | 5 / 0 |
| **accepted-correct** | 80% | 100% | 100% |
| oracle pass | 100% | 100% | 100% |
| own gate pass | 100% | 100% | 100% |
| false-green (correctness) | 0% | 0% | 0% |
| false-green (shape) | 20% | 0% | 0% |
| no change delivered | 0% | 0% | 0% |
| timeouts | 0% | 0% | 0% |
| violations, median | 0 | 0 | 0 |
| $ per run, median | 0.333 | 0.345 | 0.359 |
| tokens per run, median | 652097 | 677325 | 650590 |
| wall s per run, median | 102 | 116 | 99 |
| $ per accepted-correct | 0.417 | 0.335 | 0.371 |
| tokens per accepted-correct | 771056 | 697738 | 763871 |
| src NLOC, median | 827 | 834 | 819 |
| test NLOC, median | 510 | 510 | 510 |
| lines changed, median | 133 | 130 | 132 |
| CCN mean, median | 2.1 | 2.1 | 2.1 |
| CCN max, median | 8 | 8 | 8 |
| longest function, median | 33 | 33 | 33 |

- **B−A** accepted-correct: +20.0 points, 95% CI [+0.0, +60.0], over 1 tasks
- **C−A** accepted-correct: +20.0 points, 95% CI [+0.0, +60.0], over 1 tasks
