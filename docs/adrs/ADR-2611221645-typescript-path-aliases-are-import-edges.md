# ADR-2611221645: TypeScript path aliases are import edges

**Status:** Completed
> 2026-11-22 Completed: landed on main at f249f06; gate `cargo test -p hexa-cli --test adr_2611221645_spec --test adr_2611221645_spec_2` passed (hexa loop stage done)
> 2026-11-22 Accepted: Accepted in craigjohnson's session: he accepted the spec (re-accepted after §5 and S26), chose the fix under his rule (real bug fixes only; architectural correctness first), and its sealed tests, red before the code, pass on main
**Date:** 2026-11-22
**Drivers:** a TypeScript user's portal (React and TypeScript, 1112 graded files) imports almost everything through `@/…`, which its `tsconfig.json` maps to `./src/*`. hexa graded it F with 178 violations while the project's own alias-resolving layer test finds 508 file-to-file layer violations. A reproducer built from three real portal edges and the portal's own layer map (`a local fixture, ts-path-aliases-portal,`, commit b046c9f) has hexa report 2 violations, both from relative imports, where 5 are real. The grade is computed from part of the import graph. The owner's rule: architectural correctness first, even if a grade changes.
**Supersedes:** none. A defect in how `hexa analyze` and the code graph resolve TypeScript imports, aliased (§1-§4) and relative (§5).

## Context

`resolve_ts_import` (`hexa-analysis/src/path_normalizer.rs`) resolves a specifier that starts with `.` against the importing file and returns any other specifier as written. So `@/lib/http` stays the literal `@/lib/http.ts`: it names no project file, its layer is unknown, and an unknown layer is never a violation. The graph's TypeScript resolution (`hexa-graph/src/lib.rs`) does the same.

TypeScript resolves such a specifier through `compilerOptions.paths` in `tsconfig.json`: a pattern (`@/*`) maps to one or more targets (`./src/*`), relative to `compilerOptions.baseUrl` when it is set, else to the directory of the `tsconfig.json` that declares `paths`. A `tsconfig.json` can inherit both through `extends`. hexa already reads the pattern keys to classify imports for the import policy (ADR-2609211430), but never uses the targets.

## Decision

1. **A specifier that matches a `paths` pattern resolves to the project file it maps to.** For each pattern in the project root's `tsconfig.json` (`*` matches any text, at most once), a specifier that matches is rewritten through each of the pattern's targets in order, relative to `baseUrl` if set, else to that `tsconfig.json`'s directory, and resolves to the first that names a project file (by the extensions and index files a relative import already tries). That file is the edge's target, exactly as if the import were relative.
2. **`extends` is followed.** A `tsconfig.json` that names another by a relative path inherits its `compilerOptions.paths` and `baseUrl`; settings in the extending file win, and a `baseUrl` is relative to the file that declares it. An `extends` that names a package, not a path, is not followed.
3. **A specifier that matches no pattern, or matches one whose targets name no project file, is unchanged**: a package name stays external, as today.
4. **The code graph resolves TypeScript imports the same way**, so `hexa graph consumers` counts the files that import through an alias.
5. **A relative import resolves to the project file it names.** `resolve_ts_import` turned `../ui/Button` into `../ui/Button.ts` whether or not that file exists, so an import of a `.tsx` file or of a folder's `index.ts` named no project file: its layer was unknown, it was never a violation, and it closed no cycle. §1 resolves an alias "by the extensions and index files a relative import already tries"; a relative import now tries them: the path as written, then (for a `.js` or `.jsx` specifier, as NodeNext writes them) the `.ts` or `.tsx` file of the same stem, then the path with `.ts` and `.tsx` added, then `index.ts` and `index.tsx` inside it, and the first that names a project file is the edge's target. When none does, the target is named as before.

## Consequences

- A TypeScript project that imports through aliases is graded on its whole import graph. Its grade can fall: violations that were there and unseen are now reported. The portal reproducer goes from 2 violations to 5.
- A project with no `paths` grades as before when every relative import names a `.ts` file as written. One that imports a `.tsx` file or a folder's `index.ts` by a relative path now has those edges to the real files, so its violations, cycles and dead exports can change: they are the ones that were there and unseen (§5).

## Implementation

**Gate:** `cargo test -p hexa-cli --test adr_2611221645_spec --test adr_2611221645_spec_2`

Every phase ends with `cargo test --workspace` and `hexa analyze .`.

## References

- ADR-2609211430: the domain imports only what it is allowed (how tsconfig aliases are classified)
- `hexa-analysis/src/domain.rs` (`ImportStatement`, `ImportEdge`), `hexa-analysis/src/ports.rs` (`AstPort`)
