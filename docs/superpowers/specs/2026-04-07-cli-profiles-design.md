# CLI Profiles Design

**Date:** 2026-04-07  
**Status:** Approved

## Overview

Replace the single `~/.distri/config` credential store with a named-profile system modelled on AWS CLI. Each profile holds a fixed set of Distri environment variables (api_url, api_key, workspace_id). The active profile is tracked in `~/.distri/config`. `distri config set` is removed; all credential management moves under `distri profile`.

---

## File Format & Storage

### `~/.distri/credentials` (new)

INI format. One section per profile. Created by `distri login` or `distri profile config set`.

```ini
[default]
api_key = dak_xxx
workspace_id = 91e3e4e4-0706-4aa8-b6e3-ba64e712fa96
api_url = https://api.distri.dev/v1

[local]
api_key = dak_yyy
workspace_id = abc-123
api_url = http://localhost:8080/v1
```

Supported keys per profile: `api_key`, `workspace_id`, `api_url`. Any key may be omitted; omitted keys fall through to built-in defaults.

### `~/.distri/config` (updated)

Tracks the active profile name only. All credential keys removed.

```toml
active_profile = "local"
```

If the file has no `active_profile`, `"default"` is used.

### Migration

On first run after upgrade, if `~/.distri/credentials` does not exist and `~/.distri/config` contains `api_key` or `workspace_id`, those values are silently migrated into the `[default]` section of `~/.distri/credentials` and removed from `~/.distri/config`.

---

## Config Loading (`DistriConfig`)

**Precedence (highest → lowest):**

1. `DISTRI_API_KEY`, `DISTRI_BASE_URL`, `DISTRI_WORKSPACE_ID` env vars
2. Profile selected by `DISTRI_PROFILE` env var (overrides active_profile)
3. Profile named by `active_profile` in `~/.distri/config`
4. `[default]` profile in `~/.distri/credentials`
5. Built-in defaults (`api_url = https://api.distri.dev/v1`, others None)

**New helpers on `DistriConfig`:**

- `DistriConfig::credentials_path() -> Result<PathBuf>` — resolves `~/.distri/credentials`
- `DistriConfig::active_profile() -> String` — reads from env or config file, returns `"default"` as fallback
- `DistriConfig::load_profile(name: &str) -> Result<ProfileValues>` — parses INI, returns the named section

INI parsing is done with the `configparser` crate (already available in Rust ecosystem as `configparser = "3"`). No manual parsing.

`DistriConfig` gains no new public fields — `base_url`, `api_key`, `workspace_id` are still the resolved values after applying precedence.

---

## CLI Commands

`distri config` subcommand is **removed**. All credential management is under `distri profile`.

### `distri login [--profile <name>]`

- Runs OAuth browser flow (unchanged)
- Saves `api_key`, `workspace_id`, and `api_url` (from server response) to the named profile in `~/.distri/credentials`
- Default profile name: `"default"`
- Does **not** change `active_profile`

### `distri profile list`

Lists all profiles from `~/.distri/credentials`. Marks the active profile with `*`. Shows masked api_key (`dak_xxx...xxxx`) and workspace_id.

```
* default   api_key=dak_2Mu...fShB  workspace=91e3e4e4  url=https://api.distri.dev/v1
  local     api_key=dak_abc...1234  workspace=abc-123   url=http://localhost:8080/v1
```

### `distri profile use <name>`

Writes `active_profile = "<name>"` to `~/.distri/config`. Errors if profile does not exist in credentials file.

### `distri profile show [<name>]`

Prints all keys for the active (or named) profile. api_key masked.

### `distri profile delete <name>`

Removes the named section from `~/.distri/credentials`. Errors if the profile is currently active (user must switch first).

### `distri profile config set [--profile <name>] [--api-key <val>] [--workspace-id <val>] [--api-url <val>]`

Sets one or more keys on the active (or named) profile. Creates the profile section if it doesn't exist. At least one key flag required.

```bash
distri profile config set --api-key dak_xxx --workspace-id abc-123 --api-url http://localhost:8080/v1
distri profile config set --api-key dak_new          # update single key
distri profile config set --api-key dak_x --profile local  # target specific profile
```

### `distri profile config unset [--profile <name>] [--api-key] [--workspace-id] [--api-url]`

Removes one or more keys from the active (or named) profile. Key flags are boolean (no value). At least one key flag required.

```bash
distri profile config unset --api-url --workspace-id
```

---

## Credentials File I/O

A new module `distri-cli/src/credentials.rs` owns all reads/writes to `~/.distri/credentials`:

```rust
pub struct ProfileValues {
    pub api_key: Option<String>,
    pub workspace_id: Option<String>,
    pub api_url: Option<String>,
}

pub fn list_profiles() -> Result<Vec<(String, ProfileValues)>>
pub fn load_profile(name: &str) -> Result<Option<ProfileValues>>
pub fn save_profile(name: &str, values: &ProfileValues) -> Result<()>   // merges, preserves other keys
pub fn delete_profile(name: &str) -> Result<()>
pub fn set_active_profile(name: &str) -> Result<()>
pub fn get_active_profile() -> String
pub fn migrate_legacy_config() -> Result<()>
```

The `save_profile` function **merges** — it only updates the keys provided and leaves others untouched. This is critical for `profile config set` updating a single key without clobbering others.

---

## Error Handling

- `distri profile use <nonexistent>` → clear error: "Profile 'foo' not found. Run `distri profile list` to see available profiles."
- `distri profile delete <active>` → error: "Cannot delete the active profile. Run `distri profile use <other>` first."
- `distri profile config set` with no key flags → clap validation error before execution.
- Missing `~/.distri/credentials` → treated as empty (no profiles), not an error.

---

## Testing

- Unit tests in `credentials.rs` covering: save/load round-trip, merge behavior, list, delete, migration
- Integration: `distri profile list` on empty credentials prints helpful empty state message
- No store/DB tests needed — pure file I/O

---

## Out of Scope

- Profile inheritance (one profile extending another)
- Encrypting credentials at rest
- Per-command `--profile` flag (only `profile config set/unset` support it; `--profile` on other commands is a future addition)
