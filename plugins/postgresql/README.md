# PostgreSQL Plugin

Lightweight scaffolding for running SQL against PostgreSQL within distri workflows. It focuses on connection management patterns and placeholder execution that can be extended with a real client.

## Connection Handling

The tools look for a connection string in this order:

1. `connection_string` parameter
2. `POSTGRES_CONNECTION_STRING` secret
3. `postgres` / `postgres:default` secret keys

## Tools

| Tool | Description |
| --- | --- |
| `execute_query` | Run arbitrary SQL with optional parameters (stub executor) |
| `list_tables` | List tables in a schema using `information_schema`

Replace the stub implementation with your preferred PostgreSQL client when you wire this into a real environment.
