# Discovery — Stage 2: Infer Cross-Project Edges

You are given a collection of per-project interaction-surface records.
Your task is to propose relationship edges between catalogued components
based on what their surfaces reveal.

## Edge kinds (transitional vocabulary)

This release uses the component-ontology v2 vocabulary. The next backlog
task ships a full decision tree rendered from the shipped ontology
definition; for now, the most useful subset for the proposals you will
likely write is:

- **`generates`** (directed; lifecycle `codegen`) — A's tooling emits
  source committed to B. Evidence: A's `produces_files` matches B's
  source tree; B documents "run A to regenerate".
- **`orchestrates`** (directed; lifecycle `dev-workflow` or `runtime`) —
  A drives B's lifecycle / multi-step workflow. Evidence: A's prose
  documents driving B through phases; A reads/writes B's state files.
- **`invokes`** (directed; lifecycle `dev-workflow` or `runtime`) — A
  spawns B as a subprocess, but does not manage B's broader lifecycle.
  Weaker than `orchestrates`. Evidence: A's
  `external_tools_spawned` names B's binary.
- **`depends-on`** (directed; lifecycle `build` or `runtime`) — A
  requires B to function. Evidence: package-manifest entries; import
  statements; `consumes_files` referencing B's manifest.
- **`calls`** (directed; lifecycle `runtime`) — A is the client of an
  endpoint B serves. Evidence: A's `network_endpoints` contains an
  address B's `network_endpoints` serves.
- **`communicates-with`** (symmetric; lifecycle `runtime`) — A and B
  exchange messages over a named transport as peers. Use when no clear
  client/server split exists. Evidence: overlapping `network_endpoints`
  with matching protocol; shared `data_formats` both emit and consume.
- **`describes`** (directed; lifecycle `design`) — A documents B (docs
  repo, architecture notes, external user guide). Evidence: A's purpose
  is documentation; A's name or contents reference B.
- **`co-implements`** (symmetric; lifecycle `design`) — A and B are
  parallel implementations of the same external spec that neither
  component owns. Evidence: both declare implementing the same named
  spec; no artifact flows between them.
- **`tests`** (directed; lifecycle `test`) — A is a test harness for B
  (A's primary purpose is to exercise B). Evidence: A's purpose prose;
  A's `consumes_files` includes B's source.

For directed kinds, the canonical participant order is:
- `generates` / `orchestrates` / `invokes` / `describes` / `tests`:
  source/orchestrator/parent first, target second.
- `depends-on`: dependent first, dependency second.
- `calls`: client first, server second.

For symmetric kinds (`communicates-with`, `co-implements`), participant
order does not matter — sort the participant identifiers
alphabetically.

## Matching signals

Propose edges when you see direct evidence such as:
- Overlapping file paths or globs between one project's `produces_files`
  and another's `consumes_files` (→ `generates @ codegen`).
- Matching network endpoints (server vs. client of the same protocol/
  address) (→ `calls @ runtime` or `communicates-with @ runtime`).
- Shared data format names (same struct / schema / message type).
- Shared external tools that suggest tight coupling (e.g., both projects
  spawn `some-custom-daemon` owned by one of them) (→ `invokes` or
  `orchestrates`).
- Direct cross-project mentions in `explicit_cross_project_mentions`,
  *especially* when reciprocated by the other project.
- Documentation-style references (A's purpose is to describe B) (→
  `describes @ design`).

## Insufficient signals (do NOT propose edges from these alone)

These patterns are too weak to justify an edge on their own. Require
direct evidence from the matching-signals list above before proposing.

- **Shared upstream dependencies.** Two components independently mentioning
  the same *third* catalog component in their `explicit_cross_project_mentions`
  is NOT evidence of an edge between those two. Many unrelated components
  share infrastructure dependencies.
- **Same programming language or ecosystem.** Both being Rust crates,
  Racket packages, Swift apps, or Node packages is not a relationship.
- **Generic or trivial file-glob overlap.** Patterns like `*.txt`,
  `**/*.rkt`, `~/.config/**`, or any whole-language source-tree glob
  are too broad to constitute file-level coupling. Require a specific,
  named file or a narrow glob whose match set is plausibly produced by
  one project and consumed by another.
- **Same external tools alone.** Both components spawning `git` or `bash`
  is not evidence; both spawning a *bespoke* binary owned by one of
  them is.

When in doubt, omit the edge — false positives are costlier than missed
edges since the user reviews proposals manually.

## Evidence grade

Annotate every proposal with one of:

- **`strong`** — symmetric artifact match (A produces X, B consumes X);
  a named wire protocol both sides declare; a reciprocated explicit
  mention.
- **`medium`** — one-sided evidence; shared format name without
  location; a shared external tool that's clearly one component's
  bespoke binary.
- **`weak`** — prose overlap, purpose similarity, shared data-format
  name without location. Weak edges are permitted but must declare
  weakness.

`evidence_fields` lists the surface paths that justify the edge (e.g.
`Alpha.surface.produces_files`, `Beta.surface.consumes_files`). May be
empty only when `evidence_grade: weak` AND `rationale` justifies it
explicitly.

## Output format

Write YAML to `{{PROPOSALS_OUTPUT_PATH}}` matching this shape:

```yaml
generated_at: <ISO-8601 UTC timestamp>
proposals:
  - kind: <one of the kinds listed above, kebab-case>
    lifecycle: <one of: design | codegen | build | test | deploy | runtime | dev-workflow>
    participants: [<name>, <name>]    # see canonical order rules above
    evidence_grade: <strong | medium | weak>
    evidence_fields:
      - <e.g., "Alpha.surface.produces_files">
      - <e.g., "Beta.surface.consumes_files">
    rationale: |
      <one paragraph citing specific surface fields from the input>
```

Do NOT emit `schema_version` or `source_project_states` — those are
injected by the caller. Only propose edges between components that
appear in the input. Only use component names exactly as they appear
in the input — no paths, no aliases.

After writing the YAML, your final message should confirm the path
written. No other output is required.

## Input

The input below lists every catalogued component's surface record.

---
{{SURFACE_RECORDS_YAML}}
