# Plan status survey

You are producing a multi-project plan status overview for a developer
who wants to know which plan to run through `raveloop-cli run` next.
The plans follow the Raveloop convention: a directory is a plan iff it
contains `phase.md`; siblings `backlog.md` and `memory.md` hold task
state and distilled learnings.

Below (after the horizontal rule) are all discovered plans, grouped by
project. For each plan you have:

- project (root directory basename)
- plan (plan directory basename)
- phase (contents of `phase.md`)
- backlog (contents of `backlog.md`, or `(missing)` if absent)
- memory (contents of `memory.md`, or `(missing)` if absent)

Your output is plain text intended to be read directly in a terminal.
Produce these three sections, in this exact order, separated by blank
lines.

## Section 1 — Per-plan summary

A **space-padded, monospace-aligned table** (not Markdown pipes). Use
fixed column widths wide enough to hold the longest value in each
column plus two spaces of padding. Example:

```
PROJECT     PLAN                     PHASE         UNBLOCKED  BLOCKED  DONE  RECEIVED  NOTES
Mnemosyne   mnemosyne-orchestrator   analyse-work          3        1     2         0
Mnemosyne   sub-A-global-store       work                  8        0     0         0
Mnemosyne   sub-C-adapters           work                 26        0     0         2  2 unprocessed dispatches
```

Rules for the table:

- Header row in ALL CAPS.
- Sort by PROJECT, then PLAN.
- `UNBLOCKED` / `BLOCKED` / `DONE` are counts of backlog tasks in each
  status. Derive from `- **Status:** ...` lines.
- `RECEIVED` is the number of entries under a `## Received` heading in
  `backlog.md` that have not been triaged into numbered tasks.
- `NOTES` is a short free-text cell for anything worth surfacing
  briefly — "2 unprocessed dispatches", "backlog.md missing",
  "stale pre-pivot framing". Empty string if nothing to note.
- If a file is missing, surface that in NOTES; do not guess.

## Section 2 — Cross-project blockers

A heading `## Cross-project blockers`, followed by an indented
bulleted list. Each entry is **one blocker per bullet**, indented
two spaces, with the blocker's rationale wrapped at ~78 columns on
continuation lines indented **four** spaces so the wrapped text aligns
under the first character of the rationale (not the bullet marker).
Example:

```
## Cross-project blockers

  - `Mnemosyne/sub-G-migration` blocked on `APIAnyware-MacOS/sub-ffi-callbacks`:
    the GC-protect bug fix in APIAnyware is a prerequisite for migrating the
    affected plan files, and no alternative path exists in the current design.
```

If no cross-project blockers are detected, write:

```
## Cross-project blockers

  None detected.
```

## Section 3 — Recommended invocation order

A heading `## Recommended invocation order`, followed by up to five
numbered entries. Same indentation and wrapping discipline as
Section 2: entries indented two spaces, continuation lines wrapped at
~78 columns indented to align under the first character of the
rationale. Example:

```
## Recommended invocation order

  1. `Mnemosyne/sub-C-adapters` — has 2 unprocessed Received items that gate
     Sub-F's 28 implementation tasks; running triage promotes them into
     numbered backlog tasks and unblocks the critical path.

  2. `Mnemosyne/sub-R-knowledge-ontology` — three literature-survey tasks
     with no cross-project dependencies; cheapest unblocked work available
     and can run in parallel with Sub-C.
```

Priority order:

1. Plans with unprocessed `## Received` items whose triage unblocks
   other plans on the critical path.
2. Plans with `not_started` tasks marked `P1` and no dependencies.
3. Independent research or literature-survey plans (cheap to run,
   often unblocked).

Skip plans whose only remaining work is `done` or `blocked` on
external input.

## Rules

- Do not speculate beyond what the files say.
- Do not recommend tasks that are already marked `done`.
- When a file is missing, note it; do not infer its contents.
- Keep each rationale to one short sentence in Section 2, one or
  two sentences in Section 3.
- Never output Markdown pipe-tables (`|`); the table in Section 1
  must be space-padded monospace columns.
