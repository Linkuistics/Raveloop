# Component Relationship Ontology — Reference

**Status:** Reference specification.
**Date:** 2026-04-23.
**Applies to:** `related-components.yaml` (formerly `related-projects.yaml`),
the `ravel-lite state related-components` CLI, the Stage 2 discover prompt,
and the Rust library that implements the schema (initially
`src/ontology/`, later the standalone `component-ontology` crate).
**Supersedes:** the implicit two-kind model (`sibling`, `parent-of`) in
`src/related_projects.rs` and the Stage 2 edge-kinds section in
`defaults/discover-stage2.md`.
**Companion:** `docs/r7-related-projects-discovery-design.md` established
the current pipeline and schema v1. This document governs schema v2, the
library surface, downstream consumers, and migration.

## 1. Purpose

This document is the canonical specification of the ontology. Every
implementation artifact derives from it:

- The Rust type enum (`EdgeKind`, `LifecycleScope`, `EvidenceGrade`) matches
  §6 exactly.
- The discover Stage 2 prompt's edge-kind vocabulary matches §6 exactly.
- The `defaults/ontology.yaml` file shipped with Ravel-Lite matches §6
  exactly.

Divergence between this document and any implementation is a bug in the
implementation.

### 1.1 Why there is no migrator

`related-components.yaml` (and its v1 predecessor `related-projects.yaml`)
is an **entirely generated artifact** — Stage 2 of the discover pipeline
produces every edge. No human authors rows in it by hand; the
`add-edge`/`remove-edge` CLI is a maintenance escape hatch, not the
primary input mechanism.

Given that, schema migration would be waste: a rule-based v1 → v2
transform is *guessing* at evidence that was never captured in v1 (v1 has
no `lifecycle`, no `evidence_grade`, no `evidence_fields`), whereas a
fresh discover run under the v2 prompt produces edges with direct
evidence. The correct upgrade path is therefore: delete the v1 file and
re-run discover. See §12.

## 2. Components as the unit of relationship

The ontology describes edges between **components**. A *component* is any
addressable unit of software whose relationships to other components are
worth cataloguing.

### 2.1 What counts as a component

The ontology is deliberately unit-agnostic. Concrete examples:

- A whole *project* — a Cargo workspace, a git repository, a Node package.
  (This is the Ravel-Lite portfolio's current unit: one component per entry
  in `projects.yaml`.)
- A *crate* within a workspace — when intra-workspace coupling matters.
- A *service* in a multi-service deployment.
- A *subsystem* or bounded module within a larger project.
- An *external specification* (an RFC, a wire-protocol spec document) —
  components on either side of a `conforms-to` edge must both be
  catalogued, and the spec is one of them.
- A *third-party library* referenced by the portfolio, when relationships
  to it are worth recording.

The ontology requires only that each component have a stable,
catalog-scoped identifier. It does not require that components share a
language, repository, runtime, or ownership.

### 2.2 Naming and rationale

The earlier naming (`related-projects`, `RelatedProjectsFile`,
`projects.yaml`) was a scope marker from when the user's catalog was
exclusively whole projects. Components is the general case; projects are
one specialisation. The ontology itself changes nothing when the unit
changes — the same edge kinds apply to crates, services, or subsystems.

Rename policy (§14) retains the project catalog's filename
(`projects.yaml`) because that catalog is, literally, a list of whole
projects — widening it is a separate concern. The *edge store* is
generalised (`related-components.yaml`) because the edges themselves are
not project-specific.

### 2.3 Identifier scheme

A component identifier is an opaque string, unique within the catalog the
consumer supplies. Today the catalog is `projects.yaml` and the identifier
is the project name. A future catalog with multi-scope components (e.g.,
`service:foo`, `crate:bar`) can reuse the same edge schema without
schema changes; only the identifier format becomes richer.

The library treats identifiers as opaque: equality, ordering, and display
only. Identifier validation (existence, shape) is the catalog's
responsibility, not the ontology's.

## 3. Problem the ontology solves

Schema v1 provides exactly two edge kinds:

```rust
// src/related_projects.rs (pre-v2)
pub enum EdgeKind { Sibling, ParentOf }
```

- `sibling(A, B)` — unordered peer; shared purpose, protocol, or data format.
- `parent-of(A, B)` — ordered; A produces artifacts B consumes.

Every real cross-component coupling collapses onto one of these two
buckets, losing three orthogonal distinctions that matter:

- *When* the coupling is active — build-time codegen vs. runtime IPC vs.
  dev-workflow orchestration.
- *What* is shared — types vs. wire format vs. a whole subprocess
  lifecycle vs. a spec.
- *Which* direction the influence flows — producer → consumer vs.
  orchestrator → orchestrated vs. implementation → spec.

The concrete failure case that motivated v2: R7 smoke testing proposed
`parent-of(Ravel-Lite, Ravel)`. Ravel-Lite does not produce artifacts Ravel
links against or reads at runtime — it emits plan-state YAML *schemas*
(codegen) and spawns agents that drive the loop (dev-workflow). Two
relationships at two lifecycle scopes. The two-kind model could express
neither.

## 4. The model: three orthogonal axes

Every edge is a tuple `(kind, lifecycle, direction)` over two component
identifiers, annotated with evidence.

### 4.1 Axis 1 — `kind` (what is coupled)

Seven thematic families; 16 total kinds defined in §6.

| Family | What flows across the seam |
|---|---|
| Dependency | Transitive reachability: A needs B to function |
| Linkage | Compile/link-time symbol resolution |
| Generation | One side authors the other side's source artifacts |
| Communication | Live messages at runtime (IPC / network) |
| Orchestration | One side drives the other's lifecycle |
| Testing | One side exercises the other |
| Specification | One side defines contracts the other conforms to |

### 4.2 Axis 2 — `lifecycle` (when the coupling is active)

Seven scopes. An edge declares exactly one; multiple scopes for one pair
become multiple edges (§4.5).

| Scope | Active during | Example |
|---|---|---|
| `design` | Human authoring, shared specs | Two components implementing the same RFC |
| `codegen` | Source generation from another source | Protobuf emits structs; Ravel-Lite emits YAML schemas |
| `build` | Compilation, packaging | Library dep resolved at `cargo build` |
| `test` | Test execution | Test fixtures, mocks, integration harness |
| `deploy` | Install / provisioning | Container image, binary packaging |
| `runtime` | Live execution | RPC, shared memory, file-watch IPC |
| `dev-workflow` | Developer loop, not shipped | A tool that spawns the component under dev |

Notes:

- `design` edges are the weakest by construction — they capture "two
  independent implementations of the same spec" with no artifact flow.
- `dev-workflow` is the scope the Ravel-Lite → Ravel case lives in:
  Ravel-Lite drives Ravel during development but neither ships the other.
- `codegen` produces *source* (committed, edited, regenerated); `build`
  consumes source to produce artifacts. The distinction matters:
  Ravel-Lite → consumers is `codegen`, never `build`.

### 4.3 Axis 3 — `direction` (who-on-whom)

Direction is a property of the **kind**, not a free field:

- **Directed** kinds are order-sensitive. Canonical order is fixed per
  kind (§7).
- **Symmetric** kinds are order-insensitive. Canonicalised by sorting
  identifiers (same as v1 `sibling`).

Fixing direction per kind avoids the v1 footgun where `parent-of` had
to be documented verbally as "parent first" with no type-system
enforcement.

### 4.4 Evidence and grade

Every edge carries:

- `evidence_grade: strong | medium | weak`
- `evidence_fields: [<surface-field-reference>, …]` — Stage 1 surface paths
  the edge is grounded in (e.g., `Ravel-Lite.produces_files`,
  `Ravel.consumes_files`).
- `rationale` — free-form prose.

Grade heuristics:

- **strong** — symmetric artifact match (A produces X, B consumes X); a
  named wire protocol both sides declare; a reciprocated explicit mention.
- **medium** — one-sided evidence, shared format name without location,
  or a shared external tool that's clearly one component's bespoke binary.
- **weak** — prose overlap, purpose similarity, shared data-format name
  without location. The current Stage 2 "insufficient signals" list becomes
  the weak threshold. Weak edges are permitted but must declare weakness.

### 4.5 Multiplicity

A pair of components may have multiple edges with distinct
`(kind, lifecycle)` tuples. This is normal: Ravel-Lite ↔ Ravel is
`generates@codegen` (schemas) **and** `orchestrates@dev-workflow` (agent
loop). Two edges, two kinds, two scopes, one pair.

Dedup key: `(kind, lifecycle, canonical-participants)` — one more
dimension than v1's `(kind, canonical-participants)`.

## 5. Prior art alignment

The ontology explicitly aligns with, adopts, or departs from the
following bodies of work:

- **SPDX 3.0.1 `RelationshipType` + `LifecycleScopeType`.** Closest fit.
  We adopt the **kind × lifecycle factoring** and align kind names
  where the concept matches (correspondence table in §6). We do **not**
  adopt SPDX wholesale — its vulnerability, licensing, and bom-ref
  elements are SBOM concerns orthogonal to cross-component coupling.
- **Stevens/Myers/Constantine structured-design coupling** (1974).
  Classifies *intra-program* module coupling. Informs the surface-based
  framing (an edge is characterised by what crosses the seam) but does
  not contribute kind names — its units are functions, not components.
- **Maven scopes / Gradle configurations.** Inform the lifecycle-scope
  enum (`compile`, `runtime`, `test`, `provided-by-host`). They aren't
  edge kinds — they're lifecycle qualifiers on a single kind
  (`depends-on`).
- **CycloneDX component scope** (`required | optional | excluded`). A
  per-edge modality bit. We capture the required/optional distinction
  via the dedicated `has-optional-dependency` vs. `depends-on` kind
  pair rather than a separate field.
- **Bazel/Pants/Nx.** Reinforce that **codegen is a first-class edge**
  (Bazel's `genrule`). Maps directly to the v2 `generates` kind.

## 6. Edge-kind reference

Kind names are kebab-case. Each entry: direction · typical lifecycle(s) ·
definition · SPDX alignment · primary Stage 1 evidence.

### 6.1 Dependency family

- **`depends-on`** · directed · `build` | `runtime`
  A requires B to function at the declared scope. Library-level
  dependency with a direction.
  SPDX: `dependsOn`.
  Evidence: package-manifest entries, import statements, `consumes_files`
  referencing B's manifest.

- **`has-optional-dependency`** · directed · `build` | `runtime`
  A can function without B but gains capability when B is present.
  SPDX: `hasOptionalDependency`.
  Evidence: `optional-dependencies` manifest sections, feature flags,
  plugin discovery.

- **`provided-by-host`** · directed · `runtime`
  A expects B to be present in the execution environment, not bundled.
  SPDX: `hasProvidedDependency`.
  Evidence: "expects X in PATH", servlet-style container-provided comments.

### 6.2 Linkage family

- **`links-statically`** · directed · `build`
  A embeds B's compiled code in its own artifact.
  SPDX: `hasStaticLink`.
  Evidence: static-lib dep in build manifest.

- **`links-dynamically`** · directed · `runtime`
  A loads B at runtime (shared object, dylib, plugin).
  SPDX: `hasDynamicLink`.
  Evidence: `dlopen` calls, dynamic-lib manifest entries, plugin-loader
  config.

### 6.3 Generation family

- **`generates`** · directed · `codegen`
  A's tooling emits source that is committed to B (or to a location B
  consumes as source).
  SPDX: `generates`.
  Evidence: A's `produces_files` matches B's source tree; B documents
  "run A to regenerate".

- **`scaffolds`** · directed · `dev-workflow`
  A emits a one-shot initial structure for B that is not regenerated on
  change. The coupling ends at B's first commit.
  SPDX: (no direct equivalent — closest is `generates` with explicit
  `noLifecycleScope`).
  Evidence: `create-X` templates, cookiecutter-style tools.

### 6.4 Communication family

- **`communicates-with`** · symmetric · `runtime`
  A and B exchange messages at runtime over a named transport, as peers.
  Use when no clear client/server split exists.
  SPDX: no direct equivalent.
  Evidence: overlapping `network_endpoints` with matching protocol; shared
  `data_formats` that both emit and consume.

- **`calls`** · directed · `runtime`
  A is the client of an endpoint B serves.
  SPDX: no direct equivalent (nearest: `usesTool`).
  Evidence: A's `network_endpoints` contains an address B's
  `network_endpoints` serves.

### 6.5 Orchestration family

- **`invokes`** · directed · `dev-workflow` | `runtime`
  A spawns B as a subprocess. Distinguish lifecycle: one-shot CLI
  invocation is `dev-workflow`; persistent process management is
  `runtime`.
  SPDX: `invokedBy` (inverse).
  Evidence: A's `external_tools_spawned` names B's binary; B exports that
  binary as its primary artifact.

- **`orchestrates`** · directed · `dev-workflow` | `runtime`
  Stronger than `invokes`: A manages B's lifecycle, state, and multi-step
  workflow.
  SPDX: no direct equivalent.
  Evidence: A's prose documents driving B through phases; A reads/writes
  B's state files; reciprocated explicit mentions.

- **`embeds`** · directed · `runtime`
  A runs B in-process (library embedding, WASM, subprocess-in-pipe).
  Distinct from `links-dynamically`: B is a whole program, not a library.
  SPDX: no direct equivalent.
  Evidence: A documents embedding B's runtime.

### 6.6 Testing family

- **`tests`** · directed · `test`
  A is a test harness for B (A's primary purpose is to exercise B).
  SPDX: `hasTest` (inverse).
  Evidence: A's purpose prose; A's `consumes_files` includes B's source.

- **`provides-fixtures-for`** · directed · `test`
  A provides test data, mocks, or fixtures that B's test suite loads.
  SPDX: no direct equivalent (related: `hasInput` at `test` scope).
  Evidence: fixture file paths overlap; prose.

### 6.7 Specification family

- **`conforms-to`** · directed · `design`
  A implements a spec defined in B (protocol, schema, RFC-internal).
  SPDX: `hasSpecification` (inverse).
  Evidence: B's primary artifact is a spec document; A references it.

- **`co-implements`** · symmetric · `design`
  A and B are parallel implementations of the same external spec that
  neither component owns (two LSP clients; two MCP servers).
  SPDX: no direct equivalent (distantly: `hasVariant`).
  Evidence: both components declare implementing the same named spec; no
  artifact flows between them.

- **`describes`** · directed · `design`
  A documents B (docs repo, architecture notes, external user guide).
  SPDX: `describes` / `hasDocumentation`.
  Evidence: A's purpose is documentation; A's name or contents reference
  B.

### 6.8 What was deliberately omitted

- **`sibling`** and **`parent-of`** — the v1 kinds. Always replaced by a
  more specific v2 kind; if Stage 2 cannot pick one, the edge is not
  worth emitting.
- **`shares-types`** — reducible to `depends-on` (A imports B's type
  defs) or `generates` (B's codegen emits A's types).
- **Negative edges** ("A and B are not related") — deferred (§13).
- **Numeric confidence scores** — three evidence grades are enough for
  review-gate workflow (consistent with R7's posture).
- **Hyperedges** (3+ participants) — binary-edge invariant retained
  (§13).

## 7. Direction and symmetry reference table

| Kind | Directed? | Canonical order |
|---|---|---|
| `depends-on` | yes | dependent first |
| `has-optional-dependency` | yes | dependent first |
| `provided-by-host` | yes | dependent first |
| `links-statically` | yes | binary first, lib second |
| `links-dynamically` | yes | loader first, loaded second |
| `generates` | yes | generator first, generated second |
| `scaffolds` | yes | scaffolder first |
| `communicates-with` | **no** | sorted |
| `calls` | yes | client first, server second |
| `invokes` | yes | parent process first |
| `orchestrates` | yes | orchestrator first |
| `embeds` | yes | host first, embedded second |
| `tests` | yes | tester first, tested second |
| `provides-fixtures-for` | yes | provider first |
| `conforms-to` | yes | implementer first, spec second |
| `co-implements` | **no** | sorted |
| `describes` | yes | describer first, described second |

## 8. On-disk schema (v2)

### 8.1 File

Path: `<config-root>/related-components.yaml` (renamed from
`related-projects.yaml`; see §14).

```yaml
schema_version: 2
edges:
  - kind: generates
    lifecycle: codegen
    participants: [Ravel-Lite, Ravel]
    evidence_grade: strong
    evidence_fields:
      - Ravel-Lite.produces_files
      - Ravel.consumes_files
    rationale: |
      Ravel-Lite emits LLM_STATE/<plan>/backlog.yaml schemas that Ravel's
      runtime reads; the schema definition lives in Ravel-Lite.

  - kind: orchestrates
    lifecycle: dev-workflow
    participants: [Ravel-Lite, Ravel]
    evidence_grade: strong
    evidence_fields:
      - Ravel-Lite.external_tools_spawned
      - Ravel-Lite.purpose
    rationale: |
      Ravel-Lite spawns claude / pi agent subprocesses as part of a phase
      loop it drives; it is Ravel's dev-workflow orchestrator.

  - kind: co-implements
    lifecycle: design
    participants: [ClientA, ClientB]         # symmetric: sorted
    evidence_grade: medium
    evidence_fields:
      - ClientA.purpose
      - ClientB.purpose
    rationale: |
      Both components implement the MCP stdio spec; neither owns the spec.
```

### 8.2 Field specification

- `schema_version: 2` — integer, required, exact match.
- `edges` — list of edge records.
  - `kind` — one of the kebab-case kinds in §6. Required.
  - `lifecycle` — one of the scopes in §4.2. Required.
  - `participants` — list of exactly two component identifiers. Distinct.
    For directed kinds, ordered per §7. For symmetric kinds, sorted.
  - `evidence_grade` — `strong | medium | weak`. Required.
  - `evidence_fields` — list of `<component>.<surface-field>` strings. May
    be empty only when `evidence_grade = weak` and `rationale` justifies
    it explicitly. Non-empty otherwise.
  - `rationale` — free-form prose. Required, non-empty.

### 8.3 Dedup / canonical key

```
key(edge) = (edge.kind, edge.lifecycle, participants′)
where participants′ = sorted(edge.participants)  if edge.kind is symmetric
                    = edge.participants          otherwise
```

Two edges with equal `key` are duplicates. Idempotent inserts (same key)
are no-ops. Distinct keys on the same participant pair are legal and
expected (§4.5).

### 8.4 Conflict detection

Retain only one check: **same directed kind, reversed participants**
(e.g., both `depends-on(A, B)` and `depends-on(B, A)`) is a modelling
error at the same lifecycle and is rejected. Cross-kind "conflict" from
v1 (e.g., `sibling(A,B)` vs. `parent-of(A,B)`) is gone — multiple kinds
per pair are expected.

## 9. Rust library surface

The implementation lives initially at `src/ontology/` inside Ravel-Lite.
Extraction to a standalone crate is staged in §11.

### 9.1 In scope for the library

- Types: `EdgeKind`, `LifecycleScope`, `EvidenceGrade`, `Edge`,
  `RelatedComponentsFile`.
- `serde`-driven load / save with `schema_version` gate; atomic write
  helper (mirroring existing `save_atomic`).
- `Edge::canonical_key`, `Edge::is_directed`, `Edge::validate`.
- `RelatedComponentsFile::add_edge` with idempotent dedup.
- `rename_component_in_edges(&mut self, old, new)` — mirrors current
  `rename_project_in_edges`.
- Hard-error loader for non-v2 `schema_version` values (no upgrade
  path; see §12).
- `SCHEMA_VERSION: u32 = 2` constant.
- An optional `validate_against_ontology(ontology: &OntologyYaml)`
  helper, for callers that want drift detection between their in-code
  enum and `ontology.yaml` (§10).

### 9.2 Explicitly out of scope for the library

- Catalog integration (`projects.yaml`, component identifier
  resolution). The library treats identifiers as opaque strings.
- The discover pipeline (Stage 1 / Stage 2). These stay in Ravel-Lite;
  the library provides the types they serialise into.
- The CLI wrapper. `ravel-lite state related-components …` remains in
  Ravel-Lite, as a thin adapter.
- Prompt templates. The Stage 2 prompt is Ravel-Lite's property; the
  library may expose kind-name constants that prompts substitute in, but
  the prompt itself is not shipped by the library.
- Filesystem locations. Callers supply a path. The library has no
  opinion on `<config-root>`.

### 9.3 Dependency posture

Minimal. The library depends on `serde`, `serde_yaml`, `anyhow`,
`thiserror`. No tokio, no clap, no filesystem conventions. This is the
precondition for painless extraction.

### 9.4 Public API sketch

```rust
pub const SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EdgeKind {
    DependsOn, HasOptionalDependency, ProvidedByHost,
    LinksStatically, LinksDynamically,
    Generates, Scaffolds,
    CommunicatesWith, Calls,
    Invokes, Orchestrates, Embeds,
    Tests, ProvidesFixturesFor,
    ConformsTo, CoImplements, Describes,
}

impl EdgeKind {
    pub fn is_directed(self) -> bool { /* §7 */ }
    pub fn as_str(self) -> &'static str { /* kebab */ }
    pub fn parse(s: &str) -> Option<Self> { /* … */ }
    pub fn all() -> &'static [EdgeKind] { /* iteration */ }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LifecycleScope {
    Design, Codegen, Build, Test, Deploy, Runtime, DevWorkflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceGrade { Strong, Medium, Weak }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub kind: EdgeKind,
    pub lifecycle: LifecycleScope,
    pub participants: Vec<String>,
    pub evidence_grade: EvidenceGrade,
    #[serde(default)]
    pub evidence_fields: Vec<String>,
    pub rationale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelatedComponentsFile {
    pub schema_version: u32,
    #[serde(default)]
    pub edges: Vec<Edge>,
}
```

### 9.5 Invariants enforced by the library

- `schema_version == 2` on both read and write. Reading any other
  version (including v1) is a hard error; the library does not attempt
  in-memory upgrade. See §12 for the upgrade procedure.
- `participants.len() == 2`, `participants[0] != participants[1]`.
- For directed kinds: participants stored in semantic order.
- For symmetric kinds: participants stored sorted.
- `evidence_grade` present; `evidence_fields` non-empty unless
  `evidence_grade == Weak`.
- `rationale` non-empty.

## 10. The ontology YAML — `defaults/ontology.yaml`

A single language-neutral file ships with Ravel-Lite at
`defaults/ontology.yaml`. It is the data form of §6 + §7 + §4.2. Its
purpose is twofold:

1. **Single source of truth.** A build-time test (mirroring
   `embedded_defaults_are_valid` / the coding-style-drift test) asserts
   that the `EdgeKind` Rust enum and the YAML list agree exactly.
   Adding a kind in one place without the other fails the test.
2. **Prompt input.** `defaults/discover-stage2.md` substitutes the kind
   list into the prompt via a token (`{{ONTOLOGY_KINDS}}`) rather than
   hard-coding it in prose. Vocabulary evolves in one place.

Sketch:

```yaml
schema_version: 1   # of the ontology file itself; independent of the
                    # related-components.yaml schema_version
kinds:
  - name: depends-on
    family: dependency
    directed: true
    lifecycles: [build, runtime]
    spdx: dependsOn
    description: |
      A requires B to function at the declared scope…

  - name: co-implements
    family: specification
    directed: false
    lifecycles: [design]
    spdx: null
    description: |
      …

lifecycles:
  - name: design
    description: Human authoring, shared specs
  - name: codegen
    description: Source generation from another source
  # …

evidence_grades:
  - name: strong
    criterion: …
  # …
```

Consumers outside Ravel-Lite can parse this file without pulling in the
Rust crate.

## 11. Extraction plan

Staged. Do not attempt to extract on day one.

### 11.1 Phase A — internal module (immediate)

Location: `src/ontology/` inside Ravel-Lite. Replaces
`src/related_projects.rs`. Shape matches §9 already, but lives as a
module crate-internally. Public use within Ravel-Lite only.

Entry criteria: design approved, follow-up implementation task accepted.

Exit criteria: v2 schema in production; Stage 2 emits v2; all tests
green; at least one full discover → apply cycle produces v2 edges.

### 11.2 Phase B — workspace member crate (medium term)

Move `src/ontology/` → `crates/component-ontology/` in a Cargo
workspace. Still vendored inside Ravel-Lite's repo. No functional change;
the extraction is purely structural.

Entry criteria: Phase A has been stable for ≥1 release; a second
consumer inside Ravel-Lite (e.g., a phase prompt renderer that walks the
graph) is on the near horizon.

Exit criteria: `cargo build -p component-ontology` succeeds independently;
no Ravel-Lite-specific code in the crate; the crate has no path
dependencies on Ravel-Lite's code (only the reverse).

### 11.3 Phase C — published crate or external repo (speculative)

Publish to `crates.io`, or extract to a dedicated repo, when a second
tool outside Ravel-Lite asks for the ontology. Until then, the
workspace-local crate is sufficient and premature publication costs more
than it saves.

### 11.4 Crate API principles

- **No Ravel-Lite concepts leak.** No references to phases, plans,
  backlog, subagents, or `LLM_STATE`. The crate's universe is edges and
  components.
- **Narrow dependency footprint.** Adding a new dependency requires a
  documented reason; the smaller the footprint, the cheaper Phase C is.
- **Semver from day one.** Phase A ships as `0.x`; the API shape
  stabilises before a 1.0 cut.
- **Portability-first naming.** File paths, config keys, prompt tokens —
  none of them appear in the crate. They live in Ravel-Lite's thin
  adapter code.

## 12. Handling pre-v2 files

There is deliberately **no migrator**. `related-projects.yaml` is an
entirely generated artifact (§1.1); the correct upgrade path is to
regenerate under v2, not to transform v1 in place.

### 12.1 Upgrade procedure

1. Delete `<config-root>/related-projects.yaml` (and
   `<config-root>/discover-proposals.yaml` if present — its schema also
   bumps).
2. Run `ravel-lite state related-components discover --apply`.
3. A fresh `<config-root>/related-components.yaml` is produced, with
   every edge carrying `lifecycle`, `evidence_grade`, and
   `evidence_fields` directly from Stage 2 evidence rather than inferred
   post hoc.

The Stage 1 per-component surface cache at
`<config-root>/discover-cache/<name>.yaml` does **not** need to be
deleted — its schema is unchanged, and preserving it keeps the re-run
cheap.

### 12.2 Loader behaviour

- Reading `related-projects.yaml` at the old path: hard error with an
  actionable message pointing at §12.1.
- Reading `related-components.yaml` with `schema_version != 2`: hard
  error (consistent with existing drift behaviour for other YAML files).
  The error message names the file, the observed version, the expected
  version, and the `discover --apply` command.
- No in-memory upgrade path. No deprecation window on the v1 schema.
- Hand-authored edges (via the `add-edge` escape hatch) are the only
  content that could theoretically be lost across an upgrade. Since
  `add-edge` is not the primary population mechanism, this is a
  documented user responsibility: if a user has hand-authored edges in
  v1 that a discover re-run does not reproduce, they re-apply them with
  v2 `add-edge` invocations.

## 13. Discover pipeline changes

### 13.1 Stage 1 — non-breaking addition

`SurfaceRecord` gains one optional field:

- `interaction_role_hints: [generator, orchestrator, test-harness,
  spec-document, spawner, documented-by, …]` — advisory labels a
  component's own prose declares about itself. Stage 2 still picks the
  kind from cross-referenced evidence; hints are priors, not verdicts.

No existing field is removed or renamed. The surface-record cache key
(subtree tree SHA) is unaffected.

### 13.2 Stage 2 — prompt rewrite

`defaults/discover-stage2.md` is rewritten:

- The "Edge kinds" section is replaced by substitution of
  `{{ONTOLOGY_KINDS}}` rendered from `defaults/ontology.yaml`.
- A new "Decision tree" section explicitly walks the kind-picking order:

  ```
  1. Runtime message exchange? (network_endpoints match)
     → communicates-with | calls
  2. Source generation into another tree? (produces_files ∩ sources)
     → generates  @ codegen
  3. Process spawning? (external_tools_spawned ∩ owner)
     → invokes | orchestrates
  4. Library dependency? (manifest evidence)
     → depends-on | links-statically | links-dynamically
  5. Common external spec declared by both?
     → co-implements @ design
  6. Doc-repo relationship?
     → describes
  7. Test harness / fixture provider?
     → tests | provides-fixtures-for  @ test
  8. None of the above + no direct evidence?
     → no edge
  ```

- Output schema updates to match §8: `lifecycle`, `evidence_grade`,
  `evidence_fields`, `rationale`. Existing `rationale` and
  `supporting_surface_fields` carry over (the latter renamed to
  `evidence_fields`).

### 13.3 Apply phase

`src/discover/apply.rs`:

- Canonical-key check updated per §8.3 (add lifecycle dimension).
- Conflict detection narrowed per §8.4 (cross-kind conflicts gone;
  reversed-directed-edges check retained).
- Proposals file schema bumps in lockstep.

## 14. Consumer audit and rename policy

Every known reader / writer of the v1 graph, and what changes:

### 14.1 Direct consumers

| Site | File / symbol | v2 change |
|---|---|---|
| Core types | `src/related_projects.rs` | Moves to `src/ontology/`. Module + types renamed (`related_projects` → `ontology`, `RelatedProjectsFile` → `RelatedComponentsFile`, `rename_project_in_edges` → `rename_component_in_edges`). |
| Constant | `RELATED_PROJECTS_FILE` | Renamed `RELATED_COMPONENTS_FILE`; value `related-components.yaml`. |
| Discover Stage 2 output | `src/discover/stage2.rs`, `src/discover/schema.rs` | Emits v2 `ProposalRecord`; proposals-file `schema_version` bumped. |
| Discover apply | `src/discover/apply.rs` | Canonical-key + conflict-detection updates (§8.3–§8.4). |
| Discover cache rename cascade | `src/discover/cache.rs` | Unaffected (cache is keyed on component name; rename cascade already handled). |
| CLI | `ravel-lite state related-projects …` | Renamed `state related-components …`. Keep `state related-projects` as a deprecated alias for **one** release cycle, emitting a stderr warning that forwards to the new name. |
| CLI args | `add-edge kind a b` | Extended: `add-edge kind lifecycle a b --evidence-grade … --evidence-field … [--evidence-field …] --rationale …`. `kind` values match §6. |
| CLI args | `list [--plan]` | Extended: `list [--plan] [--kind X] [--lifecycle Y]` for filtering. |
| Rename cascade | `src/projects.rs` (`run_rename`) | Calls `rename_component_in_edges` instead of `rename_project_in_edges`; cache filename rename unchanged. |
| Legacy markdown migrator | `state migrate-related-projects` | Reads per-plan `related-plans.md` and emits edges. Since v2 loaders reject v1 files (§12.2), this CLI must either be retired at v2 cutover or taught to emit v2 edges (trivial: it already has enough context to pick `depends-on` / `describes`). Retire by default; reintroduce only if a user asks. |
| Tests | `tests/state_related_projects.rs` | Renamed + extended for the new fields. Fixture edges in existing unit tests (`src/related_projects.rs:606–1077`) become v2. |

### 14.2 Indirect consumers and non-consumers

- `src/prompt.rs` / `read_related_plans_markdown` (`src/main.rs:1083`,
  `src/multi_plan.rs:27`, `src/multi_plan.rs:62`) — reads **per-plan
  markdown** (`related-plans.md`), not the structured graph.
  **Unaffected.** This is the legacy integration that the future
  graph-aware prompt substitution will eventually replace.
- Phase prompts that reference the graph today — **none**. The v2
  schema is thus not breaking any prompt contract today; new prompts
  that consume the graph will adopt v2 directly.
- `projects.yaml` — the component catalog itself. **Unaffected**; its
  schema is independent.

### 14.3 Filename rename

- `<config-root>/related-projects.yaml` — no file moves. The v1 file is
  deleted by the user as part of the §12.1 upgrade; the v2 file is
  written by `discover --apply` at the new path `related-components.yaml`.
- `<config-root>/discover-proposals.yaml` — unchanged filename; only
  its schema bumps. Any residual v1 proposals file is deleted as part of
  the §12.1 upgrade.
- `<config-root>/discover-cache/*.yaml` — unchanged, retained across
  upgrade to keep the re-run cheap.

### 14.4 Rename policy

- CLI: deprecated alias for one release cycle, then removed.
- Types / constants / modules: no aliases. One Ravel-Lite release ships
  the rename atomically with the schema bump.
- `projects.yaml` catalog: **not renamed**. The catalog is a project
  catalog today; the ontology operating over it is a separate concern.

## 15. Open questions

1. **Hyperedges.** `orchestrates(A, {B, C, D})` is more accurate for an
   orchestrator with multiple subjects than three binary edges. Deferred;
   revisit when real examples accumulate.
2. **Temporal decay.** Edges valid at a past snapshot but no longer.
   Stage 1 already caches on tree SHA; a per-edge
   `first_seen / last_confirmed` pair could let edges age gracefully.
   Deferred.
3. **Per-kind evidence schemas.** Typed discriminated-union per kind
   (`generates` requires `produces_files ↔ consumes_files`; `calls`
   requires an endpoint match) would catch mislabelled evidence at
   validation time. Deferred until a second tool consumes the graph.
4. **Negative edges.** "A and B look related but are not" —
   suppresses repeated false-positive proposals. Could be a separate
   `excluded_edges` list.
5. **Catalog pluralism.** If the catalog ever gains non-project
   components (crates, services), component identifiers will need a
   shape beyond `projects.yaml` names. The ontology itself does not
   change; the catalog schema does.

## 16. Acceptance checklist

This reference is acceptable when implementation ships and:

- [ ] Every kind in §6 is groundable in ≥1 Stage 1 surface field.
- [ ] The Ravel-Lite → {Ravel, APIAnyware-MacOS, TestAnyware} smoke-test
      case resolves without `parent-of` and without information loss.
- [ ] A build-time drift test ensures the Rust `EdgeKind` enum and
      `defaults/ontology.yaml` stay in lockstep.
- [ ] A build-time drift test ensures the Stage 2 prompt renders the
      kind list from the ontology YAML, not from hard-coded prose.
- [ ] Loading a v1 file (either at the old `related-projects.yaml` path
      or with `schema_version: 1` at the new path) produces an actionable
      hard error that names the `discover --apply` command (§12.2). No
      silent upgrade path remains in the loader.
- [ ] SPDX-alignment column in §6 is accurate — no claimed
      correspondence that SPDX 3.0.1 doesn't actually have.
- [ ] All direct consumers in §14.1 compile against the new types;
      legacy `read_related_plans_markdown` (§14.2) is untouched.
- [ ] The extraction-readiness criteria in §11.4 hold for `src/ontology/`
      before it graduates to a workspace crate.
