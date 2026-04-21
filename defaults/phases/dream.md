You are running the DREAM phase of a multi-session backlog plan. The
dream phase runs periodically when memory.md has grown past the
compaction headroom. Its job is to rewrite memory.md losslessly in
tighter form — consolidating memories, like dreaming.

## Required reads

1. `{{PLAN}}/memory.md` — the file to rewrite
2. `{{ORCHESTRATOR}}/fixed-memory/memory-style.md` — the Memory
   style rules

## Do NOT read

Anything else. Not `backlog.md`, not `session-log.md`, not
`latest-session.md`, not `related-plans.md`. Fresh context for rewriting
means no task momentum, no session narrative — just the text and the
style rules.

## Behavior

1. Rewrite `{{PLAN}}/memory.md` **in place**, applying the Memory style
   rules from `fixed-memory/memory-style.md`.

2. Your contract is **strictly lossless**. Preserve every live fact. Only
   rewrite prose. Do not delete entries unless they are pure duplicates
   (same claim, different wording). Reflect is the only lossy-pruning
   phase; dream is not.

3. Run `ravel-lite state set-phase {{PLAN}} git-commit-dream`.

4. Stop.

If the dream produces a bad result, it is recoverable:
`git checkout memory.md` restores the prior version. `memory.md` is
always tracked in git.

## Output format

After completing the rewrite, print a brief summary using this structure.
Each entry is **two lines**: the label line carries the pre-change state;
a continuation line beginning with `→` carries the post-change state.

```
[OVERLAPPING] <heading A> + <heading B>
           → <result heading>
[VERBOSE] <heading> — <what was wordy>
       → <how it's now tightened>
[AWKWARD] <heading> — <old phrasing>
       → <new phrasing>
[STATS] <before word count>
     → <after word count>
```

Labels name the **state that caused the change**, not the action taken
(e.g. VERBOSE, not TIGHTENED). Two lines per significant change — old
on top, new under it. Minor prose edits can be omitted. End with the
STATS entry. Do not include any other commentary. Do not mention phase.md.
