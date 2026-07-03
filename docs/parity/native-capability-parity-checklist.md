# Native Capability Parity Checklist

Last updated: 2026-07-02

Status values: `Verified`, `Implemented`, `Partial`, `Missing`.

## Capability Surface UX

| Capability | Status | Native evidence | Remaining work |
| --- | --- | --- | --- |
| Web-level extension shell | Verified | `crates/studio-app/src/app/components/mod.rs` renders Assistant Tools as a real modal root over a dimmed workbench instead of pushing the timeline/transcript down; `crates/studio-app/src/app/components/native_capability_panel.rs` gives the modal fixed dimensions, internal scroll, and separate Reviewer / Gold Drafting tabs; Escape closes the modal before touching loop state | Keep future Helper/Reviewer/Gold UX additions inside modal workspaces or first-class editor interactions, not attached developer panels. |
| Web-level dialog chrome | Verified | `crates/studio-app/src/app/components/mod.rs` `view_modal_layer` renders submit, skip-chunk, admin-flag, hotkeys, task-lookup, edit-annotation, no-work, and exit-confirm dialogs as centered fixed-width modal cards over the dimmed scrim (`opaque` guards dialog-body clicks, scrim clicks route each dialog's existing cancel/outside message, no-work stays non-dismissible); hotkeys dialog scrolls internally; window title mirrors document title with project name plus `●` unsaved marker; `exit_on_close_request(false)` plus `WindowCloseRequested` guard unsaved changes behind a Keep Editing / Discard and Exit confirm (second close request force-exits); admin-flag reason input auto-focuses and submits on Enter; task-lookup inputs submit on Enter; `workbench_dialogs_render_as_centered_modal_layers_over_scrim`, `window_close_request_*`, and `window_title_reflects_project_name_and_unsaved_changes` cover the surface | Extend only as new dialogs appear; keep scrim dismissal aligned with the Escape dismissal stack. |

## Helper

| Capability | Status | Native evidence | Remaining work |
| --- | --- | --- | --- |
| Hotkeys help | Verified | `crates/studio-app/src/capabilities/mod.rs`, `crates/studio-app/src/app/components/hotkeys_dialog.rs`, rendered Helper Extensions hotkey section for selected-row actions, Right Shift segment focus navigation, playback speed, speaker workflow, rewind, text move, timeline trim/transcribe/auto-insert/auto-segment/cut-preview, SKAZ, linter, and focus rows; `hotkeys_dialog_renders_ported_helper_extension_rows` plus the broad `helper_` suite verify the recovered registry surface | Add only if future Helper capabilities introduce new verified shortcuts. |
| Row actions | Verified | `crates/studio-app/src/app/segment_actions/structure.rs`, transcript action cells, selected-row delete and merge-above/below Helper hotkeys routed to existing native row reducers; Shift+1/2 playback speed, Alt+X one-second rewind, Alt+1/2 / Alt+` speaker workflow switch/reset, and physical Right Shift + Left/Right focus navigation are native reducer actions; `helper_row_action_hotkeys_route_only_when_not_typing` and the broad `helper_` suite verify routing and input guards | Extend only if future source comparison finds another Helper row action beyond the recovered shortcut set. |
| Text move | Verified | `crates/studio-app/src/app/helper_text.rs`, `helper_text_move_*` tests | Convert direct text edits into grouped command semantics if batch undo becomes necessary. |
| Focus toggle | Verified | Native Escape/focus routing in `hotkeys.rs` and browser-input guards; focused transcript Esc emits an unfocus + Helper workflow message; reducer mirrors recovered four-state Esc pause/blur/resume/restore state and proportional restore-offset bookkeeping; transcript editors now have deterministic focus IDs, restore tasks target the exact row, and draft content cursors move to the recovered proportional offset before focus lands | Extend only if future source comparison finds focus analytics or fallback states beyond the recovered four-state workflow and proportional cursor restore. |
| Quick region autocomplete | Verified | `tag_suggestions.rs`, create-region/editor integration, recovered square-bracket tag suggestions, style open/close suggestions, selected-text style tag wrapping with undoable text-editor edits, keyboard navigation/accept/dismiss flow, and Babel `{MIS: }` / `{PRO: }` curly tag suggestions with caret placement before the closing brace | Extend only if future source comparison finds bridge behavior beyond native text editor controls. |
| Selected number to SKAZ | Verified | `helper_text.rs`, SKAZ tests, digit-over-selection row editor test | Expand only if new Helper SKAZ cases are found. |
| Timeline selection | Verified | Native loop, region, waveform drag controls; `crates/studio-app/src/app/helper_timeline.rs` ports recovered `Alt+R` current-row and `Alt+Shift+R` all-visible decoded-audio trim with inward, outward-to-silence, final cleanup, and same-lane neighbor guard passes; `Alt+Shift+G` routes recovered current-empty-segment transcription to the native drafting audio pipeline; `Alt+C` creates padded empty segments around nearest uncovered decoded speech near the caret and merges adjacent same-lane rows within the recovered 1s window while focusing the survivor; `Alt+Shift+S` splits visible decoded-audio segments on silence runs over 1000ms and trims the result; row, waveform, and auto-segmentation splits use recovered Babel smart split word-ratio text allocation; active cut previews support recovered Alt+drag creation, tiny-drag cancellation, locked body drags, edge resize with a 1s floor, `S`/`Shift+S` commit, and `L` loop using one undoable core cut command | Extend only if future source comparison finds another Helper timeline-selection path beyond the recovered shortcut and pointer surface. |
| Timeline defaults | Verified | Recovered Helper `timelineZoomDefaults` persists only `lastZoomValue`; native `persist_timeline_defaults`/hydration persists timeline zoom plus speed, track filter, collapsed lanes, muted lanes, and lane vertical zoom across reloads with `helper_timeline_defaults_persist_and_hydrate_on_reload` coverage | Extend only if future source comparison finds defaults beyond recovered zoom persistence and native timeline state. |
| Waveform tools | Verified | Native overview waveform/minimap-style seek window, waveform cache, timeline controls, recovered tooltip-label affordances, recovered waveform scale unlock range (`0.3x` to `1000x`) across editor model, slider mapping, lane state, and persistence, plus the recovered `3x` drag-edge magnifier (`180px` width, `150px` max height) during start/end trim drags | Extend only if future source comparison finds another Helper waveform tool beyond scale unlock, tooltip affordances, autoscroll, and edge-drag magnification. |
| Custom linter | Verified | `crates/studio-app/src/linter.rs` recovered rule registry/status surface; `crates/studio-app/src/app/helper_lint_autofix.rs` mirrors the recovered bridge autofix order, including tag trailing punctuation, comma-before-dash, free mid-sentence double dash, interjection normalization, polite-pronoun casing, and same-lane segment-start context; focused `helper_lint_autofix_*` tests and the broader helper suite cover current-row and all-row actions | Extend only if future source comparison finds linter bridge behavior beyond the native current-row/all-row draft operations. |
| Extended diff | Verified | Native diff toolbar/table, parity map runtime smokes | Keep recovered diff provenance explicit when adding AI review evidence. |

## Reviewer

| Capability | Status | Native evidence | Remaining work |
| --- | --- | --- | --- |
| Generate review | Verified | `/api/review/generate` request routing, generated feedback/classification parsing, native preview cards with evidence/category/template/opinion/rationale detail lines, selected card expansion beyond preview limits, selected card-to-feedback/rubric apply flow with draft autosave, analytics payload reuse, focused tests | Extend only if recovered Reviewer adds controls beyond native selection/comment/template/apply. |
| Review session | Verified | Create/load/refresh/finalize request routing, parsed native session preview, first-class finalize result preview with mode/applied-at/backend/feedback detail lines, backend version/card/suggestion rendering, recovered expand-all/collapse-all and suggestions show/hide workspace state, selected session card expansion beyond preview limits | Extend only if recovered workflow exposes finalize actions beyond apply/skip. |
| Templates | Verified | Search/match/clear/suggest/approve/reject request routing, parsed template result/session state, card selection syncs matched template/comment inputs from the session preview, full suggestion detail rendering (description/proposed text/source cards/target template), selected review session ID persists through project metadata and hydrates on reload | Extend only if recovered Reviewer adds persistent local session caches beyond backend sessions. |
| Comments | Verified | Session/card comment request routing, parsed native comment drafts, dirty tracking, recovered 500ms reviewer comment save tick, and duplicate-request guard matching the extension save chain intent | Extend only if recovered comments add conflict-resolution UI beyond current session/card drafts. |
| Analytics | Verified | `ReviewerSubmitAnalytics` builds normalized original/current payload, native category feedback snapshots, session/card note snapshots, native metadata, and queues `/api/trpc/transcriptions.submitTranscriptReviewAction` | Extend snapshots if future Reviewer rubric controls add extra native fields. |

## Gold Drafting

| Capability | Status | Native evidence | Remaining work |
| --- | --- | --- | --- |
| Status/ping | Verified | `/health` request route, parsed backend readiness, OpenRouter credit state, provider/model/tier/reasoning diagnostics, panel status rendering, focused tests | Extend only if backend adds new health fields beyond current provider diagnostics. |
| Generate rows | Verified | Request builder, recovered `/api/draft/generate/stream` route, automatic `draftSessionId`, SSE `done` extraction, reconnectable stream failure reconciliation through plain `/api/draft/generate`, optional multipart native lane audio cues, parsed preview rows/summary, recovered first-40 before/after diff preview lines with warnings, selective row apply/skip, grouped undoable apply-all | Extend only if recovered Gold overlay adds controls beyond stream progress, audio guard choices, diff preview, apply, and restore. |
| Transcribe segment | Verified | Missing-audio guard, multipart speaker-lane audio request when native waveforms are loaded, parsed request-segment preview with recovered before/after diff detail lines, inline accept action, undoable preview-target apply even if selection changes | Extend only if recovered broker adds a separate transcript-edit review UI beyond returned text. |
| Redistribute text | Verified | JSON broker request builder, parsed redistribution previews, accepted allocation apply, model-suggested adjacent sentence move apply, malformed/invalid move validation copy, before/after allocation visualization, grouped undo; recovered broker exposes only coarse accepted/calling/waiting port events beyond the response contract | Extend only if recovered drafting UI gains finer-grained redistribution diff controls. |
| Audio cues | Verified | Native decoded lane waveforms are encoded as PCM16 WAV multipart audio tracks for draft generation and segment transcription; Gold Drafting panel reports loaded/missing audio lanes with source-path diagnostics; recovered audio guard choices are ported as Retry Audio, Use Captured Audio, and Continue Text Only | Extend only if recovered extension adds new audio-capture diagnostics beyond missing/partial speaker-lane coverage. |

## Current Verification

- `cargo test -p studio-app --bin studio-app -- workbench_dialogs_render_as_centered_modal_layers_over_scrim`
- `cargo test -p studio-app window_close_`
- `cargo test -p studio-app --bin studio-app -- window_title_reflects`
- `cargo fmt --all -- --check`
- `cargo check -p studio-app`
- `cargo test -p studio-app helper_lint_autofix -- --nocapture`
- `cargo test -p studio-app helper_ -- --nocapture`
- `cargo test -p studio-app reviewer_ -- --nocapture`
- `cargo test -p studio-app drafting_ -- --nocapture`
- `cargo test -p studio-app lane_vertical_zoom -- --nocapture`
- `cargo test -p studio-app lane_control_messages_update_babel_track_state -- --nocapture`
- `cargo test -p studio-app helper_row_action_hotkeys_route_only_when_not_typing -- --nocapture`
- `cargo test -p studio-app hotkeys_dialog_renders_ported_helper_extension_rows -- --nocapture`
- `cargo test -p studio-app helper_timeline_defaults_persist_and_hydrate_on_reload -- --nocapture`
- `cargo test -p studio-app babel_transcript_editor_escape_unfocuses_and_runs_helper_focus_workflow -- --nocapture`
- `cargo test -p studio-app helper_focus_escape_workflow_matches_recovered_four_state_toggle -- --nocapture`
- `cargo test -p studio-app helper_playback_workflow_hotkeys_match_recovered_helper_routes -- --nocapture`
- `cargo test -p studio-app helper_playback_speed_rewind_and_speaker_workflow_hotkeys_update_native_editor -- --nocapture`
- `cargo test -p studio-app helper_right_shift_ -- --nocapture`
- `cargo test -p studio-app helper_trim_ -- --nocapture`
- `cargo test -p studio-app helper_timeline_transcription_hotkey_routes_to_native_drafting_segment_transcription -- --nocapture`
- `cargo test -p studio-app helper_auto_insert_ -- --nocapture`
- `cargo test -p studio-app helper_timeline_auto_insert_hotkey_routes_to_native_decoded_audio_insert -- --nocapture`
- `cargo test -p studio-app helper_auto_insert_segment_at_caret_merges_adjacent_same_lane_rows_and_focuses_result -- --nocapture`
- `cargo test -p studio-app helper_auto_segment_ -- --nocapture`
- `cargo test -p studio-app helper_timeline_auto_segmentation_hotkey_routes_to_native_silence_split -- --nocapture`
- `cargo test -p studio-app split_smart_allocates_text_by_babel_word_ratio -- --nocapture`
- `cargo test -p studio-app helper_auto_segment_visible_silences_splits_long_decoded_silence_then_trims_segments -- --nocapture`
- `cargo test -p studio-core split_segment_with_text_keeps_allocated_content_through_undo_redo -- --nocapture`
- `cargo test -p studio-core cut_lane_range -- --nocapture`
- `cargo test -p studio-app helper_cut_preview_overlay -- --nocapture`
- `cargo test -p studio-app hotkeys_dialog_registry_matches_recovered_babel_hotkeys -- --nocapture`
- `cargo test -p studio-app escape_ -- --nocapture`
- `cargo test -p studio-app browser_behavior -- --nocapture`
- `cargo test -p studio-app app_opens_on_editor_with_bundled_project_ready -- --nocapture`
- `cargo test -p studio-app apply_pending_drafts_commits_multiple_rows_as_one_undo_step -- --nocapture`
- `cargo test -p studio-app curly_tag -- --nocapture`
- `cargo test -p studio-app tag_suggestions -- --nocapture`
- `cargo test -p studio-app applying_style_tag_suggestion_wraps_selected_text_and_marks_dirty -- --nocapture`
- `cargo test -p studio-app review_backend -- --nocapture`
- `cargo test -p studio-app native_capability -- --nocapture`
- `cargo test -p studio-app native_capability_ -- --nocapture`
- `cargo test -p studio-app escape_ -- --nocapture`
- `cargo test -p studio-app reviewer_generate_success_populates_preview_and_analytics_payload -- --nocapture`
- `cargo test -p studio-app reviewer_finalize_success_populates_feedback_preview_without_session_cards -- --nocapture`
- `cargo test -p studio-app reviewer_success_responses_populate_native_session_and_template_state -- --nocapture`
- `cargo test -p studio-app reviewer_card_selection_syncs_template_and_comment_from_session_preview -- --nocapture`
- `cargo test -p studio-app reviewer_session_id_persists_to_project_metadata_and_hydrates_on_reload -- --nocapture`
- `cargo test -p studio-app reviewer_apply_selected_card_updates_feedback_draft_and_queues_save -- --nocapture`
- `cargo test -p studio-app reviewer_card_detail_lines_include_evidence_template_and_comments -- --nocapture`
- `cargo test -p studio-app reviewer_selected_card_detail_lines_include_card_beyond_preview_limit -- --nocapture`
- `cargo test -p studio-app reviewer_finalize_detail_lines_include_mode_timestamp_and_feedback -- --nocapture`
- `cargo test -p studio-app reviewer_session_workspace_expansion_matches_recovered_review_dialog -- --nocapture`
- `cargo test -p studio-app reviewer_comment_changes_autosave_on_tick_without_duplicate_requests -- --nocapture`
- `cargo test -p studio-app reviewer_comment_save_tick_matches_recovered_500ms_debounce -- --nocapture`
- `cargo test -p studio-app drafting_health_request_and_success_populate_backend_readiness -- --nocapture`
- `cargo test -p studio-app native_health_label_includes_provider_diagnostics -- --nocapture`
- `cargo test -p studio-app drafting_response_previews_rows_and_applies_or_skips_individual_edits -- --nocapture`
- `cargo test -p studio-app drafting_generate_rows_ -- --nocapture`
- `cargo test -p studio-app extracts_final_gold_drafting_payload_from_recovered_sse_stream -- --nocapture`
- `cargo test -p studio-app maps_gold_drafting_backend_routes_from_extension_client -- --nocapture`
- `cargo test -p studio-app drafting_stream_failure_queues_recovered_reconcile_request -- --nocapture`
- `cargo test -p studio-app recovered_before_after_panes -- --nocapture`
- `cargo test -p studio-app draft_row_preview_diff_label_includes_before_after_and_warnings -- --nocapture`
- `cargo test -p studio-app draft_row_preview_diff_label_marks_changed_word_tokens -- --nocapture`
- `cargo test -p studio-app draft_transcription_preview_label_marks_token_diff_against_original -- --nocapture`
- `cargo test -p studio-app accepted_redistribution_preview_applies_draft_allocations_with_undo -- --nocapture`
- `cargo test -p studio-app model_move_redistribution_preview_applies_adjacent_sentence_moves_with_undo -- --nocapture`
- `cargo test -p studio-app malformed_redistribution_move_surfaces_validation_copy_and_blocks_apply -- --nocapture`
- `cargo test -p studio-app draft_redistribution_detail_lines_include_before_after_allocations -- --nocapture`
- `cargo test -p studio-app drafting_generate_rows_attaches_native_audio_cues_when_available -- --nocapture`
- `cargo test -p studio-app drafting_audio_cue_status_reports_loaded_and_missing_lanes -- --nocapture`
- `cargo test -p studio-app draft_audio_cue_status_label_includes_missing_lane_sources -- --nocapture`
- `cargo test -p studio-app drafting_transcribe_segment_blocks_until_native_lane_audio_is_loaded -- --nocapture`
- `cargo test -p studio-app drafting_transcribe_segment_attaches_native_lane_audio_when_available -- --nocapture`
- `cargo test -p studio-app drafting_transcription_success_populates_diff_preview_for_request_segment -- --nocapture`
- `cargo test -p studio-app drafting_apply_last_rows_uses_undoable_segment_edits -- --nocapture`
- `cargo test -p studio-app drafting_apply_segment_text_uses_preview_segment_when_selection_changes -- --nocapture`
- `cargo test -p studio-app drafting_apply_segment_text_uses_selected_segment_and_undo -- --nocapture`
- `cargo test -p studio-app`
- `cargo check --workspace`
- `cargo test --workspace`
