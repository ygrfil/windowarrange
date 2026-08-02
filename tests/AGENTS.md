# Purpose

- Verify layout correctness, controller state transitions, Windows behavior, and manual ClubGG compatibility.

# Ownership

- Property tests own mathematical layout invariants.
- Integration tests own controller behavior through mock backends.
- Synthetic Windows tests own real Win32 enumeration and movement checks against test-created windows.
- Live ClubGG acceptance remains a documented manual procedure.

# Local Contracts

- Automated tests must never discover or move real ClubGG windows.
- Native tests must target only windows created by the test process and must restore or destroy them during cleanup.
- Live tests require an explicit ignored/manual entry point and visible user initiation.
- Tests must not depend on monitor resolution, locale, or table titles.
- Controller tests must verify that denied or failed moves are reported as failures rather than successful arrangements.
- Controller tests must verify that an explicit Locate command is dispatched to the selected window without coupling it to arrangement.
- Controller tests must verify stable discovered-window order, contextual poker/application modes, ordinary-window default Ignore behavior, and persistence of all states.
- Controller tests must verify process/class fallback across an ordinary window title change, immediate ordinary-window placement on Arrange and Auto enable, configurable default behavior with explicit-rule precedence, and table-order restoration after restart.
- Manual UI checks must cover automatic panel-height fitting, full background painting, visible toolbar actions, bounded poker/application panes, 1–8 tables plus ordinary-window choices visible without scrolling, and bundled-font glyph rendering.
- The automated UI regression must render eight long-title poker candidates split across active and inactive groups plus eight ordinary candidates in a compact viewport and assert that the tile, poker board, application board, and combined workspace remain within width and height bounds.
- The UI regression must independently bound the top-toolbar height so it cannot displace the workspace outside the visible client area.
- The UI regression must assert deterministic fitted heights for empty and populated snapshots, including two active plus three inactive poker tables and three ordinary windows; the final inactive row must not clip, and section descriptions or summary headers must not reintroduce unused vertical space before the first cards.
- UI tests and the manual checklist must cover poker table-number selection, same-number cancellation, and second-number swap dispatch.
- Geometry tests must verify right-side-strip containment, full work-area height, non-overlap with active poker tables, and specifically that one or two tables never cause Fill space to select a bottom band.
- Controller tests must verify that two-slot reservation defaults on for both zero and one active table, uses the same right boundary as a real two-table layout, toggles immediately while Auto is off, and persists across restart.
- Controller tests must verify that parked tables use queried minimum sizes and line up from bottom-right toward the left without overlap.
- Geometry tests must verify that layouts above four tables preserve slots 1–4 as `(1,2)/(3,4)` and extend right with vertical pairs `5/6` and `7/8`.
- Unit tests must keep the outer shell identity free of `ClubGG` so third-party poker hooks do not target the arranger.
- Unit tests must verify the shared icon retains its blue field, spade silhouette, and centered white plus.

# Work Guidance

- Prefer deterministic fake clocks and mock window backends for debounce tests.
- Property-test counts 1 through 8 plus arbitrary valid monitor sizes and aspect ratios. Counts 1–3 must match the virtual four-table cell size; counts 4+ must remain maximal.
- Keep failure output free of full external window titles.

# Verification

- `cargo test --all-targets`
- Run the manual checklist in `tests/MANUAL_CLUBGG.md` only with the user present.

# Child DOX Index

- No child documents.
