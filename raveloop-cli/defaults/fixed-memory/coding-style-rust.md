# Rust Coding Style Guidelines

## Formatting (rustfmt)
- **Edition:** 2021
- **Import Grouping:** Group imports as StdExternalCrate (stdlib, external crates, then local)
- **Import Granularity:** Crate level (merge imports from the same crate)
- **Field Init Shorthand:** Always use field init shorthand when variable name matches field name
- **Apply formatting:** Run `cargo +nightly fmt` before committing

### Function Parameters
- **Maximum arguments:** 20 parameters
- If approaching this limit, consider refactoring into a configuration struct

## Error Handling
- Use `thiserror` for error types with `#[derive(Error)]`
- Use `anyhow` for application-level error handling, and `thiserror` for library-level error types
- Provide descriptive error messages
- Avoid using `unwrap` or `expect` in production code; handle errors gracefully
- Use `?` operator for propagating errors when appropriate
- Consider using `Result` types for functions that can fail, and avoid panicking unless absolutely necessary
- When defining custom error types, include relevant context and information to aid in debugging and error handling
- When handling errors, consider the user experience and provide actionable feedback when possible, rather than just logging the error or returning a generic message

## Async Code
- Use Tokio runtime
- Prefer bounded channels over unbounded
- Avoid blocking operations in async contexts

## Dependencies
- Use workspace dependency versions to maintain consistency
- Justify any deviation from workspace versions
- Avoid unnecessary dependencies; prefer standard library and existing workspace dependencies when possible
