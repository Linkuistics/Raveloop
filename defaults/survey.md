# Plan status survey

You are producing a multi-project plan status overview for a developer
who wants to know which plan to run through `raveloop run` next.
Plans follow the Raveloop convention: a directory is a plan iff it
contains `phase.md`; siblings `backlog.md` and `memory.md` hold task
state and distilled learnings.

Below (after the horizontal rule) are all discovered plans. For each
plan you have:

- project (basename of the nearest ancestor directory containing `.git`)
- plan (plan directory basename)
- phase (contents of `phase.md`)
- backlog (contents of `backlog.md`, or `(missing)` if absent)
- memory (contents of `memory.md`, or `(missing)` if absent)

## Your output

Respond with a single YAML document matching this schema — nothing
else. No prose preamble, no Markdown code fences, no commentary.
The calling tool parses your response and owns all final formatting.

```
plans:
  - project: <string>             # project basename, as provided above
    plan: <string>                # plan directory basename
    phase: <string>               # raw contents of phase.md, trimmed
    unblocked: <int>              # count of backlog tasks: not_started with no unmet deps
    blocked: <int>                # count of backlog tasks: status=blocked OR not_started with unmet deps
    done: <int>                   # count of backlog tasks: status=done
    received: <int>               # count of dispatches under `## Received` NOT yet promoted to numbered tasks
    notes: <string>               # short free-text cell; leave empty if nothing worth noting

cross_project_blockers:
  - blocked: <project>/<plan>     # plan that is blocked
    blocker: <project>/<plan>     # plan whose output unblocks it
    rationale: |                  # one or two sentences; use `|` block scalar for safety
      Why this is blocked and what would unblock it. Free prose.
      May span multiple lines.

parallel_streams:
  - name: <string>                # short descriptive name, e.g. "Critical path"
    plans:                        # plans that make up this stream
      - <project>/<plan>
      - <project>/<plan>
    rationale: |                  # why these belong in one stream; note any
      intra-stream sequencing (gates, dependencies) vs fully
      concurrent work. Explain why this stream can run concurrently
      with other streams.

recommended_invocation_order:
  - plan: <project>/<plan>        # plan to invoke next via raveloop run
    rationale: |                  # one or two sentences of rationale
      Why run this next, grounded in the files above.
```

## Rules for populating the schema

- Include EVERY discovered plan in the `plans` list. Do not omit any.
- Sort `plans` by project, then plan name.
- `notes` is terse (one short phrase). Use it to flag things like
  "2 unprocessed dispatches", "backlog.md missing", or "stale
  pre-pivot framing". Leave it as an empty string if there's nothing
  specific to note.
- A plan with `backlog.md` missing: counts are all 0, `notes: backlog.md missing`.
- `cross_project_blockers`: only entries where blocker and blocked
  live in different projects. Same-project blockers belong in the
  plan's own backlog, not here. Omit the key or return `[]` if none.
- `parallel_streams`: group plans into sets whose work can proceed
  concurrently with the other sets. Each stream may itself be a
  sequential chain (e.g. gate-task → implementation), but streams do
  not block each other. Every recommended plan should belong to some
  stream. Omit the key or return `[]` if all work is one linear chain.
- `recommended_invocation_order`: up to five entries, highest priority
  first. Priority order:
    1. Plans with unprocessed `## Received` items whose triage unblocks
       other plans on the critical path.
    2. Plans with `not_started` tasks marked `P1` and no dependencies.
    3. Independent research or literature-survey plans (cheap to run,
       often unblocked).
  Skip plans whose only remaining work is `done` or `blocked` on
  external input.

## Hard rules

- Respond with YAML ONLY. No preamble, no conclusion, no code fences.
- Use `|` block scalars for every multiline prose field.
- Do not speculate beyond what the files say.
- When a file is missing, surface it in `notes`; do not infer contents.
