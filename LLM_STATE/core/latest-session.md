### Session 12 (2026-04-22T07:14:29Z) — Migrate plan-state files to YAML

- Ran `ravel-lite state migrate` against `LLM_STATE/core/` to convert all
  markdown plan-state files to structured YAML (`backlog.md → backlog.yaml`,
  `memory.md → memory.yaml`, `session-log.md → session-log.yaml`,
  `latest-session.md → latest-session.yaml`).
- Dry-run confirmed identical parse counts before committing: 6 backlog, 68
  memory, 11 sessions, 1 latest. Round-tripped each area via `state <area>
  list` / `show-latest` to verify end-to-end integrity.
- Created annotated tag `pre-structured-state` at `8ce34ba` and pushed it plus
  41 pending commits to `origin` before writing any YAML.
- Backlog reorganised: completed tasks archived, R7 split into R7-design (new
  research task) + R7 implementation (now depends on R7-design), one stale
  task removed.

What worked: `state migrate` parsed all production markdown cleanly. The
`--keep-originals` default left `.md` files in place, which is the correct
choice given that phase prompts still read/write `.md` directly.

What didn't / caveat: the YAML files are **preview data, not operational data**
until R6 lands. Phase prompts continue to mutate the `.md` files; the `.yaml`
files are frozen at migration-time content and will diverge. Triage should
annotate R6's description: either re-migrate immediately before the prompt
rewrite or run `state migrate --force --delete-originals` atomically as part
of R6.

Also noted: the task's deliverable documented `--plan-dir` as a named flag, but
the real CLI takes `PLAN_DIR` as a positional argument — `--plan-dir` fails.
Docs should be corrected.

What to try next: R5 (global `related-projects` edge list) unblocked; R6
(migrate phase prompts to CLI verbs) waiting on R1–R5.

No uncommitted source-file paths. All changes are plan-state files inside
`LLM_STATE/core/` reserved for the plan-state commit.
