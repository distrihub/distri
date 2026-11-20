# distri-a2a

A2A 1.0 protocol types (agent card, messages, parts, tasks, stream events, JSON-RPC) for Distri, mirroring a2aproject/a2a-rs. `schema/a2a.json` is the official A2A 1.0 JSON Schema bundle (vendored from a2a-rs, upstream v1.0.1-48-g2b93e318); `tests/schema.rs` validates what we serialize against it.

Most users should depend on `distri` instead. Use this crate only if you need direct
access to protocol primitives or serialization.

## Install

```toml
[dependencies]
distri-a2a = "0.2.4"
```

## License

MIT
