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

3. Write `git-commit-dream` to `{{PLAN}}/phase.md`.

4. Stop.

If the dream produces a bad result, it is recoverable:
`git checkout memory.md` restores the prior version. `memory.md` is
always tracked in git.

## Output format

After completing the rewrite, print a brief summary using this structure:

```
[MERGED] <heading A> + <heading B> — <result heading>
[TIGHTENED] <heading> — <what changed>
[REWORDED] <heading> — <old phrasing → new>
[STATS] <before word count> → <after word count>
```

One line per significant change. Minor prose edits can be omitted.
End with the STATS line. Do not include any other commentary. Do not
mention phase.md.
