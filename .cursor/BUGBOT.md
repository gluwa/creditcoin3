# Cursor Bugbot Guidelines

When debugging and fixing issues in this repository, please follow these guidelines:

- **Code Quality** The project adheres to high code quality standards.
- **Compilation**: The project always needs to compile as a whole. Ensure any fixes don't break the build.
- **Unit Tests**: We have many unit tests that need to work. Verify that fixes don't break existing tests.
- **Integration Tests**: We have many integration tests that need to work. Check that fixes maintain integration test compatibility.
- **CI/CD**: Our CI jobs test compilation and all tests. Always check if changes require CI configuration updates.
- **Documentation**: Don't write documentation files unless explicitly requested.
- **Commits**: Don't auto-commit or push changes. Always ask for permission first or wait to be asked.
- **Code Quality**: Always run clippy and other formatters/linters configured in the project before finalizing fixes.
- **Root Cause**: When fixing bugs, identify and address the root cause rather than just symptoms.
- **Testing**: After fixing a bug, verify the fix works and doesn't introduce regressions.
- **Security**: Never access, read, or use GitHub secrets. Do not attempt to read secrets from environment variables, configuration files, or any other source.

