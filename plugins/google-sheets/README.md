# Google Sheets Plugin

Interact with Google Sheets to create spreadsheets and write cell ranges. Designed for rapid prototyping of sheet integrations in distri.

## Authentication

Provide an OAuth access token in `context.auth_session.access_token` with the Sheets scope.

## Tools

| Tool | Description |
| --- | --- |
| `create_spreadsheet` | Create a spreadsheet with optional sheet definitions |
| `write_to_sheet` | Write values to a range using `USER_ENTERED` semantics |
