# Google Docs Plugin

Create Google Docs programmatically from distri workflows. The example implementation writes initial content after creating the document.

## Authentication

Expect an OAuth access token in `context.auth_session.access_token` with the `https://www.googleapis.com/auth/documents` scope.

## Tools

| Tool | Description |
| --- | --- |
| `create_document` | Create a Google Doc and optionally seed it with text |
