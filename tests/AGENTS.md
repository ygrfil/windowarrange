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
- Controller tests must verify stable discovered-window order, contextual poker/application modes, ordinary-window default Ignore behavior, and persistence of all states.
- Controller tests must verify process/class fallback across an ordinary window title change, immediate ordinary-window placement on Refresh and Auto enable, and table-order restoration after restart.
- Manual UI checks must cover minimum/default/maximum panel sizes, full background painting, visible rail actions, bounded poker/application panes, 1–8 tables plus ordinary-window choices visible without scrolling, and bundled-font glyph rendering.
- The automated UI regression must render eight long-title poker candidates split across active and inactive groups plus eight ordinary candidates in a compact viewport and assert that the tile, poker board, application board, and combined workspace remain within width and height bounds.
- The UI regression must independently bound the top-bar height so header layout changes cannot displace the command rail and workspace outside the visible client area.
- The UI regression must assert that the command rail and workspace allocations share the same top coordinate; section descriptions must not reintroduce vertical offsets before the first cards.
- Geometry tests must verify right-side-strip containment, full work-area height, non-overlap with active poker tables, and specifically that one or two tables never cause Fill space to select a bottom band.
- Unit tests must keep the outer shell identity free of `ClubGG` so third-party poker hooks do not target the arranger.

# Work Guidance

- Prefer deterministic fake clocks and mock window backends for debounce tests.
- Property-test counts 1 through 8 plus arbitrary valid monitor sizes and aspect ratios. Counts 1–3 must match the virtual four-table cell size; counts 4+ must remain maximal.
- Keep failure output free of full external window titles.

# Verification

- `cargo test --all-targets`
- Run the manual checklist in `tests/MANUAL_CLUBGG.md` only with the user present.

# Child DOX Index

- No child documents.
