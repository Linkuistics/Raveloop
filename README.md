# Ravel-Lite

> An orchestration loop for LLM development cycles.
> Compose. Reflect. Dream. Triage. Repeat.

Multi-agent orchestrator for backlog-driven LLM development. Supports
[Claude Code](https://claude.ai/code) and
[Pi](https://github.com/mariozechner/pi-coding-agent) as selectable
agent backends, with shared parameterized phase files and pluggable
configuration.

A single Rust binary (`ravel-lite`) with a
[Ratatui](https://ratatui.rs) TUI. See
[docs/architecture.md](docs/architecture.md) for the module layout,
message model, and agent trait.

## Phase Cycle

```
work → analyse-work → git-commit-work →
reflect → git-commit-reflect → [dream trigger?] →
dream → git-commit-dream →
triage → git-commit-triage → [continue?] → work
```

- **work** (interactive) — user steers task selection, implements task
- **analyse-work** (headless) — examines git diff, produces session
  log and commit message from ground truth
- **reflect** (headless) — distils learnings into durable memory
- **dream** (headless, conditional) — lossless memory rewrite when
  word count exceeds baseline + headroom
- **triage** (headless) — adjusts backlog, dispatches subagents for
  cross-plan propagation
- **git-commit-\*** — per-phase audit trail commits

## Principles

- **No magic.** All config, prompts, phase state, and memory are
  readable files on disk. Nothing is embedded or hidden.
- **Visible, auditable, adjustable.** Every input is a file the user
  can inspect and edit. Every state transition writes to the filesystem.
- **Agents are subprocesses.** The orchestrator spawns CLI processes,
  reads their JSON stream output, and renders progress. It never calls
  LLM APIs directly.

## Project layout

A plan always lives at `<project>/<state-dir>/<plan>` — typically
`<project>/LLM_STATE/<plan>`. Ravel-lite derives the "project" (the
subtree it controls) from this layout as `<plan_dir>/../..` — pure
path math, independent of where `.git` sits.

### Monorepo subtrees

Because the project root is derived from the plan path rather than
from `.git`, ravel-lite runs cleanly inside a monorepo subtree. Place
the plan at `<monorepo>/<path-to-subtree>/LLM_STATE/<plan>` and
ravel-lite will treat `<path-to-subtree>` as the project: all git
queries (dirty-tree checks, baseline diffs, snapshot for analyse-work)
are pathspec-scoped to the subtree, so sibling subtrees in the same
monorepo are invisible. Baseline SHAs remain repo-wide — the scope is
on the query, not the anchor.

Commit-message prefix conventions (e.g., conventional-commits rules
the monorepo enforces) are not automated — the per-commit
`commit-message.md` authored by analyse-work is the customisation
point.

## Releasing

Versions are cut with [`cargo-release`](https://github.com/crate-ci/cargo-release):

```bash
cargo install cargo-release   # one-time
cargo release patch --execute  # 0.1.0 → 0.1.1, commit + tag v0.1.1
cargo release minor --execute  # 0.1.0 → 0.2.0, commit + tag v0.2.0
cargo release major --execute  # 0.1.0 → 1.0.0, commit + tag v1.0.0
git push origin main --follow-tags
```

Omit `--execute` for a dry run. Push is intentionally separate so the
release commit and tag can be inspected locally first. Configuration
lives in `release.toml`.

`ravel-lite version` and `ravel-lite --version` print the crate version
plus `git describe --tags --always --dirty` and the UTC build timestamp,
so a running binary always identifies the commit it was built from.

## License

See [LICENSE](LICENSE).
