# Swift Coding Style Guidelines

## Formatting
- **Formatter:** swift-format (Apple's official tool)
- **Linter:** SwiftLint with project `.swiftlint.yml`
- **Language mode:** Swift 6 where supported
- **Line length:** 120 characters
- **Indentation:** 4 spaces

## Error Handling
- Use typed `throws` (Swift 6) for functions that can fail
- Define domain errors as `enum: Error` with descriptive cases and associated values
- Avoid `try!` and `try?` outside tests; use `do/catch` and provide actionable messages
- Convert error types at module boundaries; do not leak third-party errors through public APIs
- Use `Result<Success, Failure>` only when integrating with callback-based APIs

## Concurrency
- Use Swift structured concurrency: `async/await`, `Task`, `TaskGroup`
- Use `actor` for shared mutable state; mark concurrency-safe types `Sendable`
- Avoid GCD (`DispatchQueue`) in new code
- Avoid `Task.detached` unless lifetime is genuinely independent of the caller
- Annotate UI work with `@MainActor`

## Dependencies
- Use Swift Package Manager (SwiftPM)
- Apps: pin to exact versions in `Package.swift`
- Libraries: use semver ranges (`from:`)
- Justify any deviation from existing project pins

## Testing
- Use **swift-testing** (`@Test`, `#expect`) for new code
- XCTest is acceptable in legacy targets; do not mix frameworks in the same test target
- Tests must be parallelizable; avoid shared mutable global state
