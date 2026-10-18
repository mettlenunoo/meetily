# Personal fork: what we changed and why

This fork of [Meetily](https://github.com/Zackriya-Solutions/meetily) (v0.4.1, MIT) adapts it to the *Personal AI Meeting Recorder* concept: a local-first Mac app that records meetings, transcribes them, and builds a searchable, verifiable record of decisions and commitments.

Meetily already covered most of the concept. This file lists how the upstream app maps to the spec, and what this fork adds.

## How Meetily maps to the concept

| Concept requirement | Upstream Meetily v0.4.1 | This fork |
|---|---|---|
| Mic + system audio on macOS | ✅ Core Audio process taps via `cidre` (Rust) | unchanged |
| Device switching (AirPods etc.) | ✅ mic hot-swap with recovery budget | unchanged |
| Crash-safe recording | ⚠️ 30 s checkpoints, deleted after merge without checking the result | **15 s checkpoints; deleted only after the merged file's duration is verified** |
| Separate mic / system tracks | ❌ mixed mono only | **`mic.mp4` and `system.mp4` saved alongside `audio.mp4`, sample-aligned** |
| Tail of recording on stop | ⚠️ saver could drop chunks still queued at stop | **saver drains the channel fully before finalizing** |
| Local transcription | ✅ Whisper / Parakeet, Metal on Apple Silicon | unchanged |
| Summaries (local LLM, long meetings) | ✅ Ollama/built-in, chunked map-reduce | unchanged |
| Search across meetings | ⚠️ `LIKE` over transcripts only | **SQLite FTS5 over titles, transcripts, summaries, notes, action items, decisions; ranked, highlighted** |
| Action items & decisions as records | ❌ only inside summary text | **`meeting_items` table: owner, due, status, priority, evidence quote + timestamp** |
| AI claims verifiable against the transcript | ❌ | **every item's quote is matched against the transcript; timestamp comes from the match, unverified items are flagged** |
| No invented owners / dates | ❌ | **prompt + post-processing leave them empty unless stated** |
| Markdown export | Pro feature | **`meeting.md` (YAML frontmatter, summary, items, notes, timestamped transcript) written to each meeting folder** |
| Analytics | opt-in PostHog, off by default | unchanged (stays off) |

## What changed, file by file

### Recording (Phase 1 of the concept)
- `audio/incremental_saver.rs`
  - `CHECKPOINT_SECONDS = 15` (max audio a crash can lose).
  - `IncrementalAudioSaver::new_track()` for per-source tracks, using `.checkpoints-mic/` and `.checkpoints-system/` so mixed-audio cleanup can never delete unmerged track checkpoints.
  - `verify_merged_duration()` reads the merged file's duration with FFmpeg; checkpoints are kept if it is short by more than 1 s / 1 %.
  - Crash recovery (`recover_audio_from_checkpoints`) also rebuilds the separate tracks.
- `audio/pipeline.rs` — sends the aligned, unmixed mic and system windows on a second channel (`recording_sender_for_tracks`).
- `audio/recording_saver.rs` — track savers, lossless draining on stop, `track_files` in `metadata.json` and in the `recording-saved` event.
- `audio/recording_manager.rs` — wires the new channel.

A meeting folder now contains:
```
audio.mp4        mixed (what Meetily always saved)
mic.mp4          your voice only
system.mp4       everyone else
transcripts.json
metadata.json    + "track_files": ["mic.mp4", "system.mp4"]
meeting.md       readable export (after a summary is generated)
```

### Search (Phase 2/4)
- `migrations/20261018000000_add_fts_search.sql` — `search_index` FTS5 table, backfilled, kept in sync by triggers.
- `database/repositories/transcript.rs` — `search_transcripts` uses FTS5 (BM25 ranking, `**highlighted**` snippets, prefix match on the last word) and falls back to the old `LIKE` search if needed. Also fixes a panic when slicing non-ASCII transcripts.
- Results now include `source` and `audioStartTime`; the sidebar shows where the match was found and highlights the matched words.

### Action items & decisions (Phase 3)
- `migrations/20261018000100_add_action_items.sql` — `meeting_items` table (+ search triggers).
- `summary/meeting_items.rs` — extraction runs automatically after each summary, with the same model. Long meetings are processed in ≤6k-token passes. Commands: `api_get_meeting_items`, `api_list_open_action_items`, `api_update_meeting_item`, `api_create_meeting_item`, `api_delete_meeting_item`.
- Re-running a summary replaces AI items but never the ones you created or edited.
- UI: `components/MeetingDetails/MeetingItemsPanel.tsx`, shown under the summary — tick off, edit owner/due inline, add missed items, dismiss wrong ones, see the supporting quote and when it was said.

### Export
- `summary/markdown_export.rs` — `meeting.md` refreshed after summaries and item edits. Commands: `api_export_meeting_markdown`, `api_write_meeting_markdown`.

### Claude
- **Summaries with Claude (opt-in, sends the transcript to Anthropic):** pick *Claude* in model settings and enter an API key. The default is Claude Opus 5.5; Sonnet 5.5 and Haiku 5.5 are cheaper.
- **Ask Claude about your meetings (`hearken-mcp/`):** a read-only MCP server over the Hearken database. It has four tools: `search_meetings`, `list_meetings`, `get_meeting` (summary, items, notes, optional transcript with `[mm:ss]`) and `list_action_items`. It makes no network calls and never writes. Claude only sees what it looks up while answering you.
  - Install: `cargo build --release -p hearken-mcp && cp target/release/hearken-mcp ~/.local/bin/`
  - Claude Code: `claude mcp add --scope user hearken -- ~/.local/bin/hearken-mcp`
  - Claude Desktop: under `mcpServers` in `~/Library/Application Support/Claude/claude_desktop_config.json`: `"hearken": {"command": "/Users/<you>/.local/bin/hearken-mcp"}`
  - Other database: set `HEARKEN_DB=/path/meeting_minutes.sqlite`.

## Testing on the Mac

```bash
cd frontend
pnpm install
./clean_run.sh            # dev build with logs
```

Checklist:
- [ ] Record 5 min of Google Meet and 5 min of Zoom. Meeting folder has `audio.mp4`, `mic.mp4`, `system.mp4`; all three are the same length.
- [ ] `mic.mp4` contains only you; `system.mp4` only the others (wear headphones).
- [ ] Force-quit mid-recording, reopen, use transcript recovery: audio up to ~15 s before the quit is back, including both tracks.
- [ ] Generate a summary with Ollama: action items and decisions appear under it, with quotes and `[mm:ss]` times; owners/dates are empty where nobody said them.
- [ ] Search in the sidebar for a word from a summary, a note and an action item: each is found and labelled.
- [ ] `meeting.md` appears in the meeting folder and opens cleanly in a Markdown viewer.
- [ ] 60-minute meeting: summary and items complete; no memory blow-up.

## Not done yet (next phases)
- In-app audio player with click-to-seek from transcript lines and item timestamps (Meetily has no player in the meeting view).
- "All open action items" page (the backend command `api_list_open_action_items` exists).
- Speaker labels: the separate tracks give a free "Me / Others" split for transcription — the next step is transcribing `mic` and `system` separately.
- Ask questions across meetings (semantic search + chat).
- Ghanaian-accent evaluation of Whisper vs Parakeet models.
