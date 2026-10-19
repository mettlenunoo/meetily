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
- **Ask Claude about your meetings (`hearken-mcp/`):** an MCP server over the Hearken database, with no network calls. Claude only sees what it looks up while answering you.
  - Read tools: `search_meetings`, `list_meetings`, `get_meeting` (summary, items, notes, optional paged transcript with `[mm:ss]`) and `list_action_items`.
  - **Summaries on your Claude subscription:** `save_summary` and `save_meeting_items` let Claude write a meeting's summary and items back into Hearken, with no API key. In Claude Desktop, pick the **"Summarise a Hearken meeting"** prompt, or just ask. Items go through the same `hearken-core` grounding as the app's own extraction: quotes are verified against the transcript, times come from the match, placeholder owners and dates are cleared, and your own or edited items are never replaced. The previous summary is kept in `result_backup`. Claude never deletes meetings or changes transcripts, notes or audio.
  - Install: `cargo build --release -p hearken-mcp && rm -f ~/.local/bin/hearken-mcp && cp target/release/hearken-mcp ~/.local/bin/`. The `rm` matters: macOS kills a binary copied over an existing one (exit 137).
  - Claude Code: `claude mcp add --scope user hearken -- ~/.local/bin/hearken-mcp`
  - Claude Desktop: under `mcpServers` in `~/Library/Application Support/Claude/claude_desktop_config.json`: `"hearken": {"command": "/Users/<you>/.local/bin/hearken-mcp"}`
  - Other database: set `HEARKEN_DB=/path/meeting_minutes.sqlite`.

## Testing on the Mac

```bash
cd frontend
pnpm install
./build-hearken.sh --install   # release build, signed if you have an Apple Development certificate
```
(`clean_run.sh` is upstream's and doesn't build the llama-helper sidecar; `dev-gpu.sh` with `CMAKE_DISABLE_FIND_PACKAGE_OpenSSL=ON` is the dev-mode equivalent.)

**Stable signing (one-time):** Xcode → Settings → Accounts → add your Apple ID → Manage Certificates → "+" → Apple Development. `build-hearken.sh` then signs with it, and macOS keeps Hearken's permissions across rebuilds.

Checklist:
- [ ] Record 5 min of Google Meet and 5 min of Zoom. Meeting folder has `audio.mp4`, `mic.mp4`, `system.mp4`; all three are the same length.
- [ ] `mic.mp4` contains only you; `system.mp4` only the others (wear headphones).
- [ ] Force-quit mid-recording, reopen, use transcript recovery: audio up to ~15 s before the quit is back, including both tracks.
- [ ] Generate a summary with Ollama: action items and decisions appear under it, with quotes and `[mm:ss]` times; owners/dates are empty where nobody said them.
- [ ] Search in the sidebar for a word from a summary, a note and an action item: each is found and labelled.
- [ ] `meeting.md` appears in the meeting folder and opens cleanly in a Markdown viewer.
- [ ] 60-minute meeting: summary and items complete; no memory blow-up.

Phase 1 (projects and tracking):
- [ ] Create a project with people and keywords; record a meeting that mentions them. The meeting page shows "Looks like …" with the reason, and nothing is filed until you press "File it there".
- [ ] Main + extra project: the project page lists the meeting; "also linked" shows for the extra.
- [ ] A Wednesday meeting with "by Friday" shows that Friday's date; "next week" shows "(pick a date)".
- [ ] Action items page: the overdue filter, the date picker and ticking done all work; the sidebar badge counts overdue items.
- [ ] A Mac notification for overdue or due-tomorrow items (08:00–20:00, once a day per item).
- [ ] Local-only project: its meetings are missing from Claude (hearken-mcp), and a Claude/OpenAI summary of them is refused.

## Roadmap (agreed 2026-10-18)

The organising idea is that **everything belongs to a project**: recordings, Slack threads, tasks, decisions and documents. Courses live in a separate **Study** area in the UI but share the same storage, so search, Claude, Slack and reports work the same for both.

Decisions:
- A recording has **one main project, plus optional extra projects**.
- Hearken **suggests** the project (qwen; later the calendar event) and **the user confirms**. Nothing is filed silently.
- Each project has a privacy setting. **Local only** projects are invisible to `hearken-mcp`.
- Order: the Mac test checklist first, then **projects**.

| # | Phase | Contents |
|---|---|---|
| 0 | Mac test checklist | See "Testing on the Mac". Audio integrity before anything else |
| 1 | **Projects + action-item tracking**: built 2026-10-19, not yet tested on the Mac | `projects` table (`kind`: work, client, personal or course; people, Slack channels, keywords, status, privacy); meeting links (main + extras); suggest-and-confirm after each recording; project page (timeline, open actions by person, decision log, running brief, "what changed this week"); sidebar grouping; MCP tools (`list_projects`, `get_project`, `assign_meeting_to_project`); local-only projects hidden from MCP. **Tracking:** an Action items page across all meetings (filter by project, person, status, due); spoken due dates resolved to real dates from the meeting date, with the spoken text kept and editable; overdue and due-soon Mac notifications (local); MCP `update_action_item` so Claude can mark items done or move dates on request |
| 2 | **Study area and recording types**: built 2026-10-19 (mic-only mode dropped: in a classroom the system track just records silence, and changing the mixer isn't worth the risk), not yet tested | Types: meeting, lecture, interview, voice note; type picks the template and what's extracted. Lecture notes (topic outline with `[mm:ss]`, definitions, examples, formulas, "exam hints", questions, assignments as tasks under the course); bookmark hotkey while recording; audio player with click-to-seek from transcript lines and items; mic-only mode; 2–3 h sessions |
| 3 | **Documents**: built 2026-10-19 (Word via macOS `textutil`, PDF via the print dialog), not yet tested | Several documents per recording (summary, minutes, formal report, lecture notes…) from a "Generate…" menu; new templates next to the existing six; project status reports; export to Markdown, PDF and Word; MCP `save_document` |
| 4 | **Slack ↔ projects** (with desmonds-bot): built 2026-10-19 (bridge script run locally or over SSH; the bot is not yet running in Slack), not yet tested in Slack | Hearken owns the project list and the bot gets a copy; channel → project mapping (ask when unclear); the bot tags tasks, decisions and tagged threads with the project; Hearken pulls them into the project timeline; "Send to Slack" (summary + items; transcript and audio stay local) |
| 5 | **Local intelligence (qwen)**: built 2026-10-19 (vectors in a plain table compared in Rust instead of sqlite-vec; meaning search and grounded answers checked live with nomic-embed-text + qwen2.5:14b) | Semantic search (local embedding model + sqlite-vec); "Ask Hearken" chat with citations that says "not found" rather than guessing; running project brief; weekly digest; local reminders for due items |
| 6 | **Study aids**: built 2026-10-19 (checked live: qwen2.5:14b made 11 study items from a sample lecture, all 11 quotes found in the transcript) | Flashcards with Anki export; practice quizzes linked to lecture moments; course glossary; course study guides (via Claude for volume) |
| 7 | **More**: built 2026-10-19: Me/Others labels (track loudness, no second transcription), People, calendar (EventKit, opt-in), model comparison tool (the comparison itself needs your clips) | Calendar-aware recording via macOS Calendar (name, attendees, project, "start recording?"); people view ("what does Kofi owe me?"); Me/Others speaker labels from `mic.mp4`/`system.mp4`; "All open action items" page; Whisper vs Parakeet on Ghanaian-accented English |

**Who does what:** qwen2.5:14b runs automatically after every recording, is private and free, and has about 32k tokens of context (a 1 h meeting fits in one pass, a 2 h lecture just fits). Claude via `hearken-mcp` runs on request on the user's subscription with 1M context, for work across many meetings and for combining Hearken with Slack, Calendar and Gmail connectors.
