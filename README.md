# Raveloop

> An orchestration loop for LLM development cycles.
> Compose. Reflect. Dream. Triage. Repeat.

Multi-agent orchestrator for backlog-driven LLM development. Supports
[Claude Code](https://claude.ai/code) and
[Pi](https://github.com/mariozechner/pi-coding-agent) as selectable
agent backends, with shared parameterized phase files and pluggable
configuration.

**Status:** Rust implementation in `raveloop-cli/` — a single Rust
binary with a [Ratatui](https://ratatui.rs) TUI. The original
TypeScript implementation has been removed; see
`docs/superpowers/specs/2026-04-16-rust-ratatui-rewrite-design.md`
for the design spec and
`docs/superpowers/plans/2026-04-17-raveloop-rust-rewrite.md` for the
implementation plan.

## Phase Cycle

```
work → analyse-work → git-commit-work → [continue?] →
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

## Origins

Raveloop merges [LLM_CONTEXT](https://github.com/Linkuistics/LLM_CONTEXT)
(the original bash-script orchestrator for Claude Code) and its
Pi-harness fork into a single multi-agent orchestrator. The bash
scripts were rewritten in TypeScript, then rewritten in Rust as the
current implementation.

## License

See [LICENSE](LICENSE).
