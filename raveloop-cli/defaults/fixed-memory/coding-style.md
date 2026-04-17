# Coding Style Guidelines

When writing or refactoring code in this project, follow these principles:

## Development Approach
- **Use Test-Driven Development (TDD)** - This is particularly useful when working with LLMs
- Write tests first, then implement the functionality

## File Organization
- **Keep files small** - Break large files into modules with smaller, focused files
- **Each file should contain closely related functionality only**
- **File and module names must be descriptive** of their functionality

## Naming Conventions
- **Use descriptive names** for modules, functions, and variables
- **Avoid short, cryptic names** - clarity is prioritized over brevity. Long names are perfectly fine if they improve descriptiveness by removing ambiguity i.e. being fully explicit. This applies to function and variable names as well as module and file names.
- **Use uniform naming semantics** - for example, don't use BlahName and FooId if both wrap string identifiers; choose one convention and stick to it. Rename existing code to conform to the chosen convention when necessary.
- **Use consistent naming patterns** - for example, if you have a function called `get_thing`, don't have another function called `fetch_thing` that does the same thing; choose one verb and use it consistently across the codebase.

## Code Quality Principles
- **Simplicity** - Code should be as simple as possible
- **Readability** - Maximum readability is essential
- **Reusability** - Code should be reusable and composable
- **Single Concern** - Code should be decomplected (handle only one concern)
- **Testability** - All code should be designed to be testable
