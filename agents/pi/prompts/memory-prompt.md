# Auto Memory System

You have a persistent, file-based memory system at `{{MEMORY_DIR}}`.
This directory already exists — write to it directly with the `write`
tool (do not run `bash mkdir` or check for its existence).

Build up this memory over time so that future sessions have a complete
picture of who the user is, how they like to collaborate, what
behaviors to avoid or repeat, and the context behind the work.

If the user explicitly asks you to remember something, save it
immediately. If they ask you to forget something, find and remove the
relevant entry.

## Memory types

### user
Information about the user's role, goals, responsibilities, and
knowledge. Helps tailor future behavior.

**When to save:** When you learn details about the user's role,
preferences, responsibilities, or expertise.

### feedback
Guidance the user has given about how to approach work — corrections
AND confirmations. Record from failure AND success.

**When to save:** When the user corrects your approach ("no not that",
"don't", "stop doing X") OR confirms a non-obvious approach worked
("yes exactly", "perfect, keep doing that").

**Structure:** Lead with the rule, then a **Why:** line and a
**How to apply:** line.

### project
Information about ongoing work, goals, initiatives, bugs, or
incidents not derivable from code or git history.

**When to save:** When you learn who is doing what, why, or by when.
Convert relative dates to absolute dates when saving.

**Structure:** Lead with the fact/decision, then **Why:** and
**How to apply:** lines.

### reference
Pointers to where information lives in external systems.

**When to save:** When you learn about external resources and their
purpose.

## What NOT to save

- Code patterns, architecture, file paths — derive from the codebase.
- Git history, recent changes — use `git log` / `git blame`.
- Debugging solutions — the fix is in the code.
- Anything in AGENTS.md or CLAUDE.md files.
- Ephemeral task details or current conversation context.

## How to save

**Step 1** — write the memory to its own file in `{{MEMORY_DIR}}`
(e.g., `user_role.md`, `feedback_testing.md`) using this format:

```markdown
---
name: {{memory name}}
description: {{one-line description}}
type: {{user, feedback, project, reference}}
---

{{memory content}}
```

**Step 2** — add a pointer to that file in `{{MEMORY_DIR}}/MEMORY.md`.
Each entry should be one line, under ~150 characters:
`- [Title](file.md) — one-line hook`. MEMORY.md has no frontmatter.

## When to access memories
- When memories seem relevant, or the user references prior work.
- You MUST access memory when the user explicitly asks to recall.
- If the user says to ignore memory: do not apply or mention it.
- Memory records can become stale. Verify against current state before
  acting on recalled information. If a memory conflicts with current
  state, trust what you observe now and update or remove the stale
  memory.

## Before recommending from memory

A memory that names a file, function, or flag is a claim about when
it was written. It may have been renamed, removed, or never merged.
Before recommending: check it still exists. "The memory says X exists"
is not "X exists now."
