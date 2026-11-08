# Security policy

## Reporting a vulnerability

Please report a vulnerability privately, not in a public issue or pull request.

Use GitHub's private vulnerability reporting: on the repository page choose
**Security → Report a vulnerability**. Include what you found, the version
(`hexa --version`), and the smallest steps that reproduce it.

You can expect an acknowledgement, a statement of whether it is accepted as a
vulnerability, and a fix or a stated reason it will not be fixed. Please allow
time for a fix before disclosing publicly.

## Supported versions

hexa is released under calendar versions (`26.11.5`). Only the latest release
is supported; a fix ships as a new release.

## What hexa guards, and what it does not

hexa is one local binary. It has no daemon, no database and no network peer,
and all of its state is files. Inference goes to the provider or local model
endpoint you configure.

What it guards:

- **File writes stay inside the repository.** Every file write is checked
  against path traversal (`emit::resolve_in_repo`).
- **Secrets are never written.** API keys are environment variables, read when
  a request is made. They are not committed and not written to any file hexa
  produces. `.env` files are never committed; use `.env.example`.
- **Agent changes are gated.** An agent's edit is committed only if the command
  you named exits 0, and no agent commit can carry an edit to a sealed test.
- **No unescaped HTML in generated browser code.** Primary adapters must not
  use `innerHTML`, `outerHTML` or `insertAdjacentHTML` with data from outside
  the domain layer.

What it does not: hexa runs the commands you give it and the agent you
configure, with your permissions. Run it on code and with models you trust, and
review what an agent changes before you merge it.

## In scope

- Writing outside the repository, or following a path out of it.
- A secret or credential reaching a file, a commit, or a log.
- A gate that passes when it should not, such as an edit to a sealed test
  getting through.
- A flaw in the code hexa scaffolds that makes the scaffolded project unsafe.

## Out of scope

- A malicious model, endpoint or command that you configured yourself.
- Findings that need an attacker who already controls your machine or account.
