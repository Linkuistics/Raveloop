# TypeScript Coding Style Guidelines

## Formatting & Linting
- **Tool:** Biome (single tool for format + lint); fall back to Prettier + ESLint in repos already using them
- **Modules:** ESM only; no CommonJS in new code
- **TS config baseline:** `strict: true`, `noUncheckedIndexedAccess: true`, `exactOptionalPropertyTypes: true`
- **Imports:** prefer named imports; avoid default exports for non-component modules

## Type System
- No `any`; use `unknown` and narrow at boundaries
- Prefer `interface` for public object shapes, `type` for unions and computed types
- Use `readonly` and `as const` aggressively for immutable data
- Validate external data with a schema library (zod, valibot) at the boundary, not via type assertions

## Error Handling
- Throw only at system boundaries (entrypoints, network, parsing)
- Use discriminated unions or a `Result` type for domain failures: `{ ok: true, value } | { ok: false, error }`
- Define error subclasses; never throw plain strings or objects
- Preserve causes: `throw new MyError("...", { cause: e })`
- Avoid `try/catch` as control flow; let errors bubble to a single handler per layer

## Async Code
- Always `await` Promises; lint with `no-floating-promises`
- Use `AbortController` for cancellation; thread the signal through every async API
- `Promise.all` for parallel work; `Promise.allSettled` when individual failures are tolerable
- Avoid `Promise.race` for timeouts; use a timeout combinator with explicit cleanup

## Runtime
- Runtime-agnostic by default (Node, Bun, Deno) — flag any runtime-specific API explicitly
- Target the minimum runtime version the project commits to in `engines` and `tsconfig.target`

## Dependencies
- Use the package manager already configured; default to **pnpm** for new projects, **bun** when speed is a priority
- Lockfile must be committed
- Avoid `@types/*` packages when the library ships its own types

## Testing
- **vitest** for new projects; jest is acceptable in existing repos
- Co-locate tests as `*.test.ts` next to source
- Prefer dependency injection over module-resolution mocking
