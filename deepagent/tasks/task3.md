# Task 3 — Directory Analyzer

Workspace: `code_samples/test-project3`

Deliverables:
- Implement `scanDirectory(root, options)` in `src/scanner.js` returning file and directory statistics.
- Support optional glob ignore patterns via `options.ignore` (minimally using `minimatch`-style matching).
- Implement `src/cli.js` to print readable JSON summary and accept multiple `--ignore` flags.
- `npm test` must pass, and `node src/cli.js fixtures --ignore "**/*.tmp"` should exclude `.tmp` files from counts.
