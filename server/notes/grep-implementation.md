# Grep Implementation Reference

## Using the `grep` crate directly

We should use the `grep` crate instead of building our own search functionality. This provides a robust, well-tested implementation.

## Key References

- **Simple grep example**: https://github.com/BurntSushi/ripgrep/blob/master/crates/grep/examples/simplegrep.rs
- **Documentation**: https://docs.rs/grep/latest/grep/
- **GitHub repository**: https://github.com/BurntSushi/ripgrep

## Implementation Notes

The `grep` crate provides:
- `grep-searcher`: Core search functionality
- `grep-matcher`: Pattern matching (regex, literals)
- `grep-regex`: Regex-based matching
- `grep-printer`: Output formatting (we may not need this for our use case)

## Benefits

1. **Battle-tested**: Used by ripgrep, one of the fastest grep implementations
2. **Feature-rich**: Supports all grep features we need
3. **Performance**: Highly optimized
4. **Maintenance**: Well-maintained by the ripgrep team

## Usage Pattern

Based on the simplegrep example:
1. Create a matcher (regex or literal)
2. Create a searcher with configuration
3. Use searcher to search files/content
4. Collect results via sinks or callbacks

## Integration Strategy

- Replace our custom search implementation with direct `grep` usage
- Use `grep-searcher` and `grep-regex` for file content search
- Keep our result structures but populate them using grep callbacks
- Scope every search to the active workspace by resolving file roots from `CURRENT_WORKING_DIR` (defaults to `examples/` when running `distri-server` locally). This keeps the CLI, backend, and UI in sync about which files are editable via the Files workspace.
