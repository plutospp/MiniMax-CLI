# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Model picker now auto-discovers models from the active provider's models API (`/v1/models`, OpenAI- and Anthropic-compatible) and falls back to the built-in MiniMax catalog when discovery fails
- `/login [provider]` slash command — enter a provider API key via masked input; saves to `[providers.<name>].api_key` (top-level `api_key` for `minimax`), preserving config formatting, and hot-reloads the engine client when the target is the active provider
- Assistant messages now render Markdown in the transcript (headings, bold/italic/strikethrough, inline code, links, bullet/ordered lists, blockquotes, rules) instead of showing raw syntax; fenced code blocks keep syntax highlighting
- Markdown tables render as aligned, width-fitted columns (bold header, rule line, per-column alignment) instead of overflowing raw pipe text
- Transcript scrollbar column is always present so mouse-wheel scrolling and scrollbar click/drag work from any position, without layout reflow at the bottom boundary
- Duo coach verification backed by RLM: RLM tools (`rlm_load`/`rlm_exec`/`rlm_status`/`rlm_query`) are now registered in Duo mode (gated by `[features] rlm`), `duo_coach` accepts a `files` list that preloads files into the RLM context store, and the coach prompt requires per-checklist-item evidence citations against loaded contexts
- `/coach [model|off]` slash command — run the Duo coach on a separate model while the player keeps the active model; the setting persists in settings.toml, applies live (engine op), and `duo_coach` executes the coach turn directly against the configured model (temperature/max tokens from `[duo]`) instead of returning the prompt

## [0.6.0] - 2026-01-29

### Added
- `review` CLI subcommand — run code reviews from the command line (`minimax review --staged`, `minimax review --base origin/main`)
- `exec` CLI subcommand — headless agentic execution (`minimax exec "prompt" --auto`)
- `setup` CLI subcommand — bootstrap MCP config and skills directories (`minimax setup --mcp --skills --all`)
- `mcp` CLI subcommands — manage MCP servers (`minimax mcp init/list/connect/tools`)
- `think` tool — transparent reasoning pass-through for agent modes
- `/copy` slash command — copy last assistant message (or Nth message) to clipboard
- Updated modes display with new CLI commands

## [0.4.1] - 2026-01-28

### Added
- Search overlay can now jump to a selected result on Enter
- Command completer overlay renders live suggestions with command hints

### Changed
- Error hints display friendly labels in the transcript
- Suggestion engine tracks input and RLM context, and uses a configurable token limit

### Fixed
- Search overlay state syncing and selection clamping
- Number highlighting regex and suggestion display formatting
- TUI warning cleanup and picker lifetime signatures
- Consolidated duplicate auto-compaction paths that ran with conflicting settings
- ESC key now clears queued draft when cancelling a loading request
- /retry command now queues messages when engine is busy instead of bypassing queue
- Context % meter now recalculates after user messages, estimates tool tokens, and supports MiniMax-Text-01/Coding-01 models

## [0.4.0] - 2026-01-27

### Added
- **Shell Mode**: Press Ctrl-X to toggle between Agent and Shell mode
  - Execute shell commands directly without AI processing
  - Visual SHELL badge in header when active
  - Cross-platform support (macOS, Linux, Windows)
  - Applies command safety analysis and sandboxing when available
- New slash commands:
  - `/debug` - Show comprehensive debug information (session, context, settings)
  - `/reload` - Reload configuration from disk without restarting
  - `/usage` - Display API usage and quota information
- Enhanced `/compact` command with manual compaction (`/compact now`)
- Keyboard shortcuts:
  - Ctrl-X - Toggle shell mode
  - Ctrl-J - Insert newline for multiline input
- Updated help view with all new keyboard shortcuts

### Changed
- Header widget now shows shell mode status
- Improved status messages for configuration commands
- Context compaction now persists and updates the UI context meter

## [0.3.0] - 2026-01-25

### Added
- Auto-compaction enabled by default with token estimation and code detection
- Engine caching for system prompts and tool schemas
- Status footer with process, todo, and recent file indicators

### Changed
- CLI mode/branding descriptions for MiniMax + Coding API modes
- Footer density improvements: context-aware scroll hints, brief token usage, and condensed recent files

## [0.2.2] - 2026-01-20

### Added
- Recursive RLM sub-call support with depth limits and tool-aware prompts
- Feature flag system with CLI toggles and config support
- Execpolicy check subcommand and sandbox runner
- Responses API proxy for controlled upstream access

### Changed
- New MiniMax palette and header bar for consistent UI styling
- Shared todo/plan state across agent runs

### Fixed
- Apply-patch now tracks multi-hunk line drift more reliably

## [0.2.1] - 2026-01-19

### Added
- Paste-burst detection to keep multi-line pastes from submitting mid-stream

### Fixed
- Preserve newlines in pasted text (clipboard + bracketed paste)
- UTF-8-safe cursor edits for TUI input and API key entry
- Respect configured default model for Anthropic-compatible clients

## [0.2.0] - 2026-01-19

### Added
- Background shell task lifecycle tools (`exec_shell_wait`, `exec_shell_kill`, `exec_shell_interact`)
- Shared background exec manager for stable task IDs across tool calls

### Fixed
- UTF-8-safe truncation for tool output, hooks, RLM previews, and session titles
- Patch tool respects `\ No newline at end of file` markers
- Reject empty search strings in `edit_file` to avoid unintended replacements

## [0.1.9] - 2026-01-17

### Added
- API connectivity test in `minimax doctor` command
- Helpful error diagnostics for common API failures (invalid key, timeout, network issues)

## [0.1.8] - 2026-01-16

### Added
- Renderable widget abstraction and modal view stack for TUI composition
- Parallel tool execution with lock-aware scheduling
- Interactive shell mode with terminal pause/resume handling

### Changed
- Tool approval requirements moved into tool specs
- Tool results are recorded in original request order

## [0.1.7] - 2026-01-15

### Added
- Duo mode (player-coach autocoding workflow)
- Character-level transcript selection

### Fixed
- Approval flow tool use ID routing
- Cursor position sync for transcript selection

## [0.1.6] - 2026-01-14

### Added
- Auto-RLM for large pasted blocks with context auto-load
- `chunk_auto` and `rlm_query` `auto_chunks` for quick document sweeps
- RLM usage badge with budget warnings in the footer

### Changed
- Auto-RLM now honors explicit RLM file requests even for smaller files

## [0.1.5] - 2026-01-14

### Added
- RLM prompt with external-context guidance and REPL tooling
- RLM tools for context loading, execution, status, and sub-queries (rlm_load, rlm_exec, rlm_status, rlm_query)
- RLM query usage tracking and variable buffers
- Workspace-relative `@path` support for RLM loads
- Auto-switch to RLM when users request large file analysis (or the largest file)

### Changed
- Removed Edit mode; RLM chat is default with /repl toggle

## [0.1.0] - 2026-01-12

### Added
- Initial alpha release of MiniMax CLI
- Interactive TUI chat interface
- MiniMax M2.1 API integration (Anthropic-compatible)
- Tool execution (shell, file ops, media tools)
- MCP (Model Context Protocol) support
- Session management with history
- Skills/plugin system
- Cost tracking and estimation
- Hooks system and config profiles
- Example skills and launch assets

[Unreleased]: https://github.com/Hmbown/MiniMax-CLI/compare/v0.4.1...HEAD
[0.4.1]: https://github.com/Hmbown/MiniMax-CLI/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/Hmbown/MiniMax-CLI/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/Hmbown/MiniMax-CLI/compare/v0.2.2...v0.3.0
[0.2.2]: https://github.com/Hmbown/MiniMax-CLI/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/Hmbown/MiniMax-CLI/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/Hmbown/MiniMax-CLI/compare/v0.1.9...v0.2.0
[0.1.9]: https://github.com/Hmbown/MiniMax-CLI/compare/v0.1.8...v0.1.9
[0.1.8]: https://github.com/Hmbown/MiniMax-CLI/compare/v0.1.7...v0.1.8
[0.1.7]: https://github.com/Hmbown/MiniMax-CLI/compare/v0.1.6...v0.1.7
[0.1.6]: https://github.com/Hmbown/MiniMax-CLI/compare/v0.1.5...v0.1.6
[0.1.5]: https://github.com/Hmbown/MiniMax-CLI/compare/v0.1.0...v0.1.5
[0.1.0]: https://github.com/Hmbown/MiniMax-CLI/releases/tag/v0.1.0
