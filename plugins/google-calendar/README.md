# Google Calendar Plugin

Full-featured Google Calendar toolkit for distri integrations, including event management, free/busy lookups, and connection diagnostics.

## Authentication

Pass an OAuth access token with Calendar scopes inside `context.auth_session.access_token`. The plugin metadata documents the scopes required for read and write access.

## Tools

| Tool | Description |
| --- | --- |
| `list_events` | Fetch upcoming events with optional date filters |
| `create_event` | Create a calendar event and invite attendees |
| `update_event` | Update event metadata or timing |
| `delete_event` | Delete an event |
| `get_event` | Fetch details for a single event |
| `list_calendars` | List calendars available to the user |
| `free_busy` | Query busy windows for multiple calendars |
| `test_connection` | Validate the OAuth token by fetching the primary calendar |
