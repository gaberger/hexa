# ADR-2611062141: `hexa spec amend` records a reviewed edit to a sealed test

**Status:** Accepted
**Date:** 2026-11-06
**Drivers:** strata's stage A ADR (id 2611062020, 2026-11-06). A breaking base-schema change made two attributes required, so the fixtures in two sealed spec test files had to change: sites, racks, devices, cables, prefixes and addresses now carry a `lifecycle/status`; device types a vendor and slug; devices a role. All scenarios still pass (32 of 32 and 38 of 38), but both seals report "changed since sealing", and hexa offers no supported way to say that the change was deliberate and reviewed. The owner asked for the verb to be built (2026-11-06).

## Context

A sealed test file is recorded in the spec JSON as `sealed: {file: sha256}` (ADR-2610221540 §6). `hexa spec verify` fails when a file's hash differs, and `hexa spec seal` refuses to re-seal a changed file, with the message "A sealed test changes through the spec, not by re-sealing". The supported route is to amend the scenarios, have a person accept the spec again, run `hexa spec tests` to regenerate the test files with a model, and seal. That route is right when a scenario's meaning changes. It is disproportionate when the meaning is unchanged and only the data the tests build has to follow a schema change: it rewrites thousands of lines that were reviewed once, and it asks a model to reproduce them.

The only other way to make `verify` pass is to write the new sha256 into the spec JSON by hand. That is what the guard is there to stop: an edit made, and the seal moved to match it, with no record that anyone decided it.

## Decision

1. **A new verb, `hexa spec amend <ADR> --reason "<why>"`**, for the case where the scenarios are unchanged and a sealed file's content has to change.
2. **It shows before it records.** The first call prints a unified diff between the sealed version of each changed file and the current file, then prints the command to confirm, ending in `--confirm <sha256 of the current file>`. It writes nothing. The sealed version is the revision of that file in git history whose sha256 equals the sealed hash; if no revision matches, the verb refuses ("the sealed version is not in git history, so the edit cannot be shown"), because an amendment nobody can read is not reviewed.
3. **It records only on `--confirm`.** The call with `--confirm <hash>` succeeds only if `<hash>` equals the sha256 of the file as it is now, so the confirmation is for exactly the content that was shown. It appends to the spec JSON, per file, an entry in a new `amendments` list: `{file, from, to, by, reason, at, base_commit}`, and sets `sealed[file]` to `to`. `by` defaults to `git config user.name`, as `accept` does. An empty `--reason` is refused.
4. **What it refuses.**
   - The spec is not accepted, or its accepted hash no longer matches its scenarios (the scenarios changed: that goes through the spec).
   - A scenario's test function, named in the spec, is no longer in the new file.
   - The file is not under a `tests/` directory (the existing seal rule).
   - Nothing differs from the seal (nothing to amend).
5. **Amendments stay visible.** `hexa spec verify` prints, per ADR, how many amendments its sealed files carry (`2 amendments`), and `--verbose` lists each with its reason. `verify --ci` passes on an amended seal, and fails if an amendment's `to` does not equal the file's current hash (an edit after the amendment is drift again).
6. **`hexa spec seal` points here.** Its refusal message adds: "if the scenarios are unchanged and the edit is deliberate, `hexa spec amend`".

What the verb cannot do is judge whether an edit weakened an assertion. The diff and the reason are the review; the amendment record is the evidence that a person was shown the change.

## Consequences

- A fixture-only change to a sealed test no longer needs regeneration, and no longer tempts anyone to move the hash by hand.
- Every edit to sealed content is in the spec JSON, in git, with who, why and from which hash to which, so a reviewer can audit all of them with one grep.
- The `amendments` list grows. It is append-only; nothing deletes an entry.
- An agent can run both calls of `amend`. The record names `by` and the reason, and the change is in the diff of the spec JSON under review; it is not an authorization.

## Gate

Written before the code. From the repository root:

```
cargo test -p hexa-cli --test spec_amend && hexa spec verify ADR-2611062141
```

The tests of `spec_amend` run `amend` in a temporary git repository with an accepted, sealed spec:

- a fixture edit with the scenarios unchanged: the first call prints the diff and writes nothing (the spec JSON is byte-identical), the call with the right `--confirm` records one amendment and `verify` passes with `1 amendment`;
- a `--confirm` hash that is not the file's current hash is refused and writes nothing;
- an empty `--reason` is refused;
- a scenario's test function deleted from the file is refused;
- changed scenarios (the accepted hash no longer matches) are refused with a pointer to the spec route;
- a sealed version that is not in git history is refused;
- a file edited again after the amendment fails `verify --ci`;
- the tests count the amendments they checked and fail if they checked none.

## Applies-To: hexa-cli/src/commands/spec_ledger.rs, hexa-cli/src/commands/spec.rs

## References

- ADR-2610221540 §6, §8: sealed tests and `verify`
- strata's stage A ADR (id 2611062020): the schema change that needs this
- `hexa-cli/src/commands/spec_ledger.rs`: `seal_files` (the refusal) and `verify`
