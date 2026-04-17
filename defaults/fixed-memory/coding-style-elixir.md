# Elixir Coding Style Guidelines

## Formatting & Linting
- **Formatter:** `mix format` — gate in CI
- **Linter:** Credo with `--strict`
- **Type analysis:** Dialyzer (via Dialyxir) on every CI run; add `@spec` to public functions

## Naming & Modules
- snake_case for functions and variables; PascalCase for modules
- One module per file; file path mirrors module name (`MyApp.Foo.Bar` → `lib/my_app/foo/bar.ex`)
- Predicate functions end in `?`; bang variants (`!`) raise on failure
- Provide both `parse/1` (returns `{:ok, _} | {:error, _}`) and `parse!/1` (raises) when failure is recoverable

## Error Handling
- Use tagged tuples for expected outcomes: `{:ok, value}` / `{:error, reason}`
- Reserve `raise` for programmer errors and unrecoverable conditions
- Use `with` for chaining `{:ok, _}`-returning calls; avoid deeply nested `case`
- Define domain exceptions with `defexception`; include actionable `:message`

## Concurrency & OTP
- Build process trees with **OTP supervision** (`Supervisor`, `DynamicSupervisor`)
- No naked `spawn`; use `Task.Supervisor.start_child/2` for ad-hoc work
- Use `GenServer` for state, `Task` for short-lived work, `Agent` only for trivial state
- Set explicit timeouts on every `GenServer.call/3`
- Name only singletons; pass PIDs for everything else

## Dependencies
- Pin versions in `mix.exs`; commit `mix.lock`
- Review every transitive dependency added — Elixir's small standard library makes deps load-bearing

## Web Framework
- When the project is web-facing, assume **Phoenix 1.7+ with LiveView**
- Phoenix Contexts wrap business logic; controllers and LiveViews stay thin

## Testing
- **ExUnit** with `async: true` by default; opt out only when shared mutable state requires it
- Use `Mox` for behaviour-based mocking; avoid runtime monkey-patching
- Property-based tests via **StreamData** for parsing and validation logic
