# Configuration

MiniMax CLI reads configuration from a TOML file plus environment variables.

## Where It Looks

Default config path:

- `~/.minimax/config.toml`

Overrides:

- CLI: `minimax --config /path/to/config.toml`
- Env: `MINIMAX_CONFIG_PATH=/path/to/config.toml`

If both are set, `--config` wins. Environment variable overrides are applied after the file is loaded.

## Profiles

You can define multiple profiles in the same file:

```toml
api_key = "PERSONAL_KEY"
default_text_model = "MiniMax-M2.5"

[profiles.work]
api_key = "WORK_KEY"
base_url = "https://api.minimax.io"
```

Select a profile with:

- CLI: `minimax --profile work`
- Env: `MINIMAX_PROFILE=work`

If a profile is selected but missing, MiniMax CLI exits with an error listing available profiles.

## LLM Providers

You can define multiple chat backends and switch between them.

```toml
provider = "minimax"

[providers.minimax]
api = "anthropic"
url = "https://api.minimax.io"
api_key = "YOUR_MINIMAX_KEY"
default_model = "MiniMax-M3"

[providers.openai]
api = "openai"
url = "https://api.openai.com/v1"
api_key = "YOUR_OPENAI_KEY"
default_model = "gpt-4.1"
```

- `provider` (string): active provider name (default `minimax`).
- `[providers.<name>].api`: `anthropic` or `openai` (aliases: `openai-compat`).
- `[providers.<name>].url`: API base URL.
- `[providers.<name>].api_key`: credential for that provider.
- `[providers.<name>].default_model`: model used when switching to the provider.

**Backward compatible:** if `[providers]` is omitted, top-level `api_key` + `base_url` + `default_text_model` form an implicit `minimax` Anthropic provider.

**URL rules:**

- `anthropic`: POST `{url}/v1/messages`. MiniMax hosts without `/anthropic` use `{url}/anthropic/v1/messages`.
- `openai`: POST `{url}/chat/completions` when `url` ends with `/v1`, otherwise `{url}/v1/chat/completions`.

**Model discovery:** the `/model` picker queries the active provider's models list endpoint (`GET {url}/v1/models` for both API flavors) and offers whatever that provider serves, so newly released models appear without CLI updates. If the endpoint is missing, unreachable, or returns nothing, the picker falls back to the built-in MiniMax catalog and shows why in the transcript.

Select the active provider with:

- Config: `provider = "openai"`
- CLI: `minimax --provider openai`
- Env: `MINIMAX_PROVIDER=openai`
- TUI: `/provider` or `/provider openai`

Set a provider's API key interactively with `/login [provider]` in the TUI (masked input, defaults to the active provider). The key is written to `[providers.<name>].api_key` — top-level `api_key` for `minimax` — and the client reloads immediately when the target is the active provider.

Image/video/TTS and the Coding API remain MiniMax-only (still use top-level `api_key` / `base_url` / `api_key_2`).

## Environment Variables

These override config values:

- `MINIMAX_API_KEY`
- `MINIMAX_BASE_URL`
- `MINIMAX_PROVIDER`
- `MINIMAX_OUTPUT_DIR`
- `MINIMAX_SKILLS_DIR`
- `MINIMAX_MCP_CONFIG`
- `MINIMAX_NOTES_PATH`
- `MINIMAX_MEMORY_PATH`
- `MINIMAX_ALLOW_SHELL` (`1`/`true` enables)
- `MINIMAX_MAX_SUBAGENTS` (clamped to `1..=5`)
- `MINIMAX_AUTO_COMPACT` (`1`/`true` enables)
- `MINIMAX_COMPACTION_TOKEN_THRESHOLD` (integer, min `1`)
- `MINIMAX_COMPACTION_MESSAGE_THRESHOLD` (integer, min `1`)
- `MINIMAX_COMPACTION_KEEP_RECENT` (integer, min `1`)
- `MINIMAX_COMPACT_PROMPT` (string)
- `MINIMAX_AUTO_COMPACT_TOKEN_LIMIT` (integer, min `1`)

## Key Reference

### Core keys (used by the TUI/engine)

- `api_key` (string, required for legacy/implicit MiniMax): must be non-empty (or set `MINIMAX_API_KEY`), unless provided under `[providers.*]`.
- `base_url` (string, optional): defaults to `https://api.minimax.io` (the CLI derives the text endpoint as `<base_url>/anthropic` for MiniMax).
- `provider` (string, optional): active `[providers]` entry name; defaults to `minimax`.
- `providers` (table, optional): named LLM backends (`api`, `url`, `api_key`, `default_model`).
- `default_text_model` (string, optional): defaults to `MiniMax-M2.5` for the legacy MiniMax provider.
- `allow_shell` (bool, optional): defaults to `false`.
- `max_subagents` (int, optional): defaults to `5` and is clamped to `1..=5`.
- `skills_dir` (string, optional): defaults to `~/.minimax/skills` (each skill is a directory containing `SKILL.md`).
- `mcp_config_path` (string, optional): defaults to `~/.minimax/mcp.json`.
- `notes_path` (string, optional): defaults to `~/.minimax/notes.txt` and is used by the `note` tool.
- `retry.*` (optional): retry/backoff settings for API requests:
  - `[retry].enabled` (bool, default `true`)
  - `[retry].max_retries` (int, default `3`)
  - `[retry].initial_delay` (float seconds, default `1.0`)
  - `[retry].max_delay` (float seconds, default `60.0`)
  - `[retry].exponential_base` (float, default `2.0`)
- `compaction.*` (optional): automatic/manual context compaction settings:
  - `[compaction].enabled` (bool): override auto-compaction on/off
  - `[compaction].token_threshold` (int): explicit estimated-token threshold
  - `[compaction].model_auto_compact_token_limit` (int): model-aware threshold override
  - `[compaction].message_threshold` (int, default `30`)
  - `[compaction].keep_recent` (int, default `6`)
  - `[compaction].model` (string): optional model override for summarization
  - `[compaction].cache_summary` (bool, default `true`)
  - `[compaction].compact_prompt` (string): custom summarization instruction
- `hooks` (optional): lifecycle hooks configuration (see `config.example.toml`).

### Parsed but currently unused (reserved for future versions)

These keys are accepted by the config loader but not currently used by the interactive TUI or built-in tools:

- `default_image_model`, `default_video_model`, `default_audio_model`, `default_music_model`
- `output_dir`
- `tools_file`
- `memory_path`

## Runtime State Persistence

MiniMax CLI persists background runtime metadata in the workspace so it survives restart/reload:

- Background shell jobs: `<workspace>/.minimax/state/background_jobs.json`
- Sub-agent registry: `<workspace>/.minimax/state/subagents.json`

Restore behavior:

- State is restored on engine startup, session sync (including resume/load/reset flows), and `/reload`.
- Jobs that were previously `running` are restored as `orphaned` with an explicit reason because process handles cannot be reattached after restart.
- Sub-agents that were previously `running` are restored as `failed` with reason `interrupted: previous MiniMax session ended before completion`.
- Existing non-running entries remain until cleaned (`/jobs clean`, `/subagents clean`, or corresponding tools).
- Missing/corrupt state files are handled softly; the UI receives `Runtime state warning: ...` status messages instead of crashing.

## Notes On `minimax doctor`

`minimax doctor` checks default locations under `~/.minimax/` (including `config.toml` and `mcp.json`). If you override paths via `--config` or `MINIMAX_MCP_CONFIG`, the doctor output may not reflect those overrides.
