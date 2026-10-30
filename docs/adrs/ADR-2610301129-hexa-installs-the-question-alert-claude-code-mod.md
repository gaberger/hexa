---
id: ADR-2610301129
status: accepted
date: 2026-10-30
---
# ADR-2610301129: hexa installs the question-alert Claude Code mod on setup

**Status:** Accepted
**Date:** 2026-10-30
**Drivers:** On 2026-10-30, while building strata through hexa, the operator lost track of questions the agent asked at the end of long status replies. They asked for questions to stand out "in another color with background highlights", and for hexa to set that up itself. A Claude Code mod, `question-alert`, was written and validated in the strata session. It shows a yellow band above the prompt when the agent's reply ends with a question, and a "DECISION NEEDED" bar over the question dialog. It exists only in that session's mods folder. Every new hexa user and machine would have to rebuild it by hand.

## Context

hexa already writes the Claude Code side of a project: `hexa dev init` writes `.claude/settings.json` from `hexa-cli/assets/templates/hexa-claude-settings.json`, and the skills, agents and hooks from `hexa-cli/assets/` (ADR-2026-03-22-1522). A Claude Code mod is different. It is a plugin, `.claude-plugin/plugin.json` plus `hooks/hooks.json` plus a TypeScript hooks module, and it loads from an installed plugin, not from project settings. The Claude CLI installs a plugin from a marketplace. A marketplace can be a local folder holding `.claude-plugin/marketplace.json`. It is registered with `claude plugin marketplace add <folder>`, and a plugin is installed from it with `claude plugin install <plugin>@<marketplace> --scope user`. An entry with a relative `source` is read from that folder itself, so updating the folder updates the plugin.

## Decision

1. **hexa ships its Claude Code mods as embedded assets.** `hexa-cli/assets/claude-mods/` holds `.claude-plugin/marketplace.json`, a marketplace named `hexa`, and one plugin folder per mod, starting with `question-alert/`. The marketplace lists each mod with `"source": "./<mod>"`.
2. **A port names what hexa needs from the Claude CLI.** `hexa-core/src/ports/claude_mods.rs` declares `pub trait ClaudePlugins` with:
    * `fn available(&self) -> bool`: the `claude` binary can be run;
    * `fn installed(&self) -> Result<Vec<String>, String>`: the ids of installed plugins, as `<plugin>@<marketplace>`;
    * `fn add_marketplace(&self, folder: &std::path::Path) -> Result<(), String>`;
    * `fn install(&self, id: &str) -> Result<(), String>`, at user scope.
3. **The domain decides the steps, and is idempotent.** `hexa-core/src/domain/claude_mods.rs` declares `pub enum Step { Skip(String), AddMarketplace(std::path::PathBuf), Install(String) }` and `pub fn install_steps(available: bool, installed: &[String], marketplace: &std::path::Path, mods: &[&str]) -> Vec<Step>`. When `available` is false, the steps are exactly one `Skip`, whose reason names `claude`. When every mod is already installed as `<mod>@hexa`, the steps are empty. Otherwise the steps are one `AddMarketplace(marketplace)` followed by one `Install("<mod>@hexa")` for each mod not yet installed, in the order of `mods`. A mod installed from another marketplace does not count as installed.
4. **One function runs the steps through the port.** `hexa-core/src/ports/claude_mods.rs` declares, beside the trait (the domain may not import a port), `pub fn apply(plugins: &dyn ClaudePlugins, marketplace: &std::path::Path, mods: &[&str]) -> Vec<(Step, Result<(), String>)>`. It computes `install_steps` from `plugins.available()` and `plugins.installed()`, runs each step through the port, and reports every step with its outcome. A step that fails does not stop the ones after it. If `installed()` fails, it is treated as nothing installed. It never panics.
5. **Setup installs the mods and never fails because of them.** `hexa bootstrap` and `hexa dev init` both extract `hexa-cli/assets/claude-mods/` to `~/.hexa/claude-mods/`, overwriting the files hexa owns there, then call `apply` with that folder and `["question-alert"]`. They print one line per step, and a skip or a failed step is a warning. Neither command's exit status depends on the mods.

## Consequences

- Every hexa setup has questions that stand out, with no hand-built mod, and an edit to the embedded mod reaches users on their next `hexa bootstrap` or `hexa dev init`.
- hexa now calls the Claude CLI during setup. On a machine without `claude` it says so and carries on.
- Plugin install is user scope, so one install covers every project on the machine. A user can remove it with `claude plugin uninstall question-alert@hexa`, and hexa adds it again on the next setup.

## Gate

`hexa spec verify ADR-2610301129`: the spec's sealed tests, each red before the code (ADR-2610221540 §5).

## Applies-To: hexa-core/src/ports/claude_mods.rs, hexa-core/src/domain/claude_mods.rs, hexa-cli/assets/claude-mods/, hexa-cli/src/commands/bootstrap/, hexa-cli/src/commands/init.rs

## References

- ADR-2026-03-22-1522: the embedded asset bundle.
- ADR-2610221540: spec, red tests, seal.
- The operator's request of 2026-10-30, made while building strata through hexa.
