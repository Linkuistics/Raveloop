You are running the COMPACT phase of a multi-session backlog plan. The
compact phase runs periodically when memory.md has grown past the
compaction headroom. Its job is to rewrite memory.md losslessly in
tighter form.

## Required reads

1. `{{PLAN}}/memory.md` — the file to rewrite
2. `{{DEV_ROOT}}/LLM_CONTEXT/fixed-memory/memory-style.md` — the Memory
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
   phase; compact is not.

3. Write `triage` to `{{PLAN}}/phase.md`.

4. Stop.

If the compaction produces a bad result, it is recoverable:
`git checkout memory.md` restores the prior version. `memory.md` is
always tracked in git.
