# ClickHouse Plugin

Utilities for testing ClickHouse integrations from distri without depending on the full runtime. The module performs HTTP queries against the native REST endpoint.

## Connection Handling

Configuration values are resolved from parameters first, then from secrets:

- `CLICKHOUSE_HOST`
- `CLICKHOUSE_PORT`
- `CLICKHOUSE_USERNAME`
- `CLICKHOUSE_PASSWORD`

Defaults are `localhost:8123` and the `default` user with no password.

## Tools

| Tool | Description |
| --- | --- |
| `execute_query` | Run a SQL query and return the parsed JSON payload |
| `show_tables` | List tables in a target database |
