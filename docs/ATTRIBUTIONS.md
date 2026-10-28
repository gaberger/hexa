# Attributions

## Elephant-Goldfish Model (EGM)

`hexa design` and `hexa drift` run the Elephant-Goldfish Model:

- **Project:** Elephant-Goldfish Model (EGM), <https://github.com/google/elephant-goldfish-model>
- **Copyright:** Google LLC
- **Licence:** Apache-2.0. The licence text is vendored at `hexa-cli/assets/egm/LICENSE`.
- **Pinned commit:** `a8380ab5f0aba392c97e31c79f9a6666b3f09f14`
- **Method:** Dave Rensin, "Elephants, Goldfish, and the New Golden Age of Software Engineering".

EGM's reviewer personas (`agents/*.md`), its document standards
(`resources/document_standards/*.md`), `resources/execution_instructions.md`,
and the skills hexa runs (`egm-brainstorm`, `egm-draft`, `egm-review`,
`egm-resolve` and `egm-drift-report`) are copied byte for byte into
`hexa-cli/assets/egm/`. `hexa-cli/assets/egm/UPSTREAM.json` records the
repository, the commit and every file's sha256, and hexa refuses to run a
design or drift command when the copy does not match its pin. They are not
edited in place: updating EGM means a new pin.

hexa adds its own framing around those prompts. It runs each goldfish as one
tool-less completion, and it runs the elephant without a person to ask. Every
report hexa writes from an EGM prompt names the persona and the pinned commit
that produced it (ADR-2610271736).
