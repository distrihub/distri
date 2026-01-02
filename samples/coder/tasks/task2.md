# Task 2 — CSV Statistics Reporter

Workspace: `code_samples/test-project2`

Deliverables:
- Implement `src/stats.js` with async helpers `parseCsv(path)` and `summarize(path)` per README.
- Implement `src/cli.js` that prints summary JSON when passed a CSV path.
- Ignore blank lines and `#` comments; report helpful errors.
- `npm test` must pass and `node src/cli.js sample.csv` should output the summary for the provided fixture.
