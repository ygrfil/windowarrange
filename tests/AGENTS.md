# Purpose

- Verify layout correctness, controller state transitions, Windows behavior, and manual ClubGG compatibility.

# Ownership

- Property tests own mathematical layout invariants.
- Integration tests own controller behavior through mock backends.
- Synthetic Windows tests own real Win32 enumeration and movement checks against test-created windows.
- Live ClubGG and LDPlayer acceptance remains a documented manual procedure.

# Local Contracts

- Automated tests must never discover or move real ClubGG or LDPlayer windows.
- Native tests must target only windows created by the test process and must restore or destroy them during cleanup.
- Live tests require an explicit ignored/manual entry point and visible user initiation.
- Tests must not depend on monitor resolution, locale, or table titles.
- Controller tests must verify that denied or failed moves are reported as failures rather than successful arrangements.
- Controller tests must verify that an explicit Locate command is dispatched to the selected window without coupling it to arrangement. Native Locate coverage must verify that its simple raise flags permit owned-window Z-order changes and activation; live ClubGG behavior remains a manual check.
- Controller tests must verify stable discovered-window order, contextual poker/application modes, ordinary-window default Ignore behavior, typed spatial poker slots, and persistence of all states.
- Controller tests must verify process/class fallback across an ordinary window title change, immediate ordinary-window placement on Arrange and Auto enable, configurable default behavior with explicit-rule precedence, and table-order restoration after restart.
- Manual UI checks must cover automatic panel-height fitting, full background painting, visible toolbar actions, a desktop-proportional poker mirror with active tiles, muted Placeholders badges, no parked-table ghosts, named interactive parked miniatures at their actual positions, one lobby-count card directly below the mirror, bounded poker/application panes, 1–8 tables plus ordinary-window choices visible without scrolling, the separate Settings window, and shared painter-drawn icon controls.
- The automated UI regression must render long-title poker candidates, mirrored slots, ignored poker controls, and eight ordinary candidates in a compact viewport and assert that the poker board, application board, and combined workspace remain within width and height bounds.
- The UI regression must independently bound the top-toolbar height so it cannot displace the workspace outside the visible client area.
- The UI regression must assert deterministic fitted heights for empty and populated snapshots, including two active plus three inactive poker tables and five ordinary windows; final poker and ordinary-window rows must retain bottom clearance under DPI scaling and native size rounding, and section descriptions or summary headers must not reintroduce unused vertical space before the first cards.
- The UI regression must render the expanded Settings controls at their maximum independent-viewport height and prove they fit without scrolling.
- UI and controller tests must cover poker/ordinary/lobby body-click Locate dispatch and prove that full tile/card bounds are clickable after subtracting only exact action-control and number-badge rectangles, including margins and gaps around the controls. Cover direct lobby-button hit handling, sequential per-lobby Locate order, and the invariant that lobby Locate performs no resize or reposition and does not change saved Park mode. Also cover non-overlapping enlarged number-badge selection, same-number cancellation, the table-1-to-placeholder-2 move path, individual same-client swaps, whole ClubGG/LDPlayer column swaps, and the exclusion of parked tables from mirror slots and swap destinations.
- Geometry and controller tests must verify right-side-strip containment, full work-area height, non-overlap with active poker tables, and specifically that one or two tables never cause Fill space to select a bottom band. Cover a remaining strip narrower than an ordinary window's reported minimum and require the window to be moved into that exact strip after poker is laid out.
- Controller tests must verify that Preserve table slots defaults on, never adds anonymous geometry beyond two total columns, lets LDPlayer claim a reserved column, repairs the legacy middle-gap form, automatically suppresses at two or more active columns, remains manually editable, never overrides manual Off, and compacts immediately while Auto is off when manually disabled.
- Win32 and controller tests must classify untitled, exact-title, and explicitly lobby-titled ClubGG surfaces as ancillary lobbies, default all of them to Park, aggregate them into one count card with group mode commands, exclude them from slots and preservation, and still recognize real table titles as likely tables.
- Controller tests must verify that parked tables release their slot ownership, compact the remaining active assignments, appear only as actual-position miniatures, use queried minimum sizes, and line up from top-right toward the left without overlap. Parked lobbies must retain their bottom-right-to-left placement.
- Controller tests must verify that native screen moves update and persist UI slot order without immediate reflow, a closed table leaves an anonymous placeholder even when it vacates a whole column, unaffected tables retain their slots, and a subsequently opened table consumes the hole through normal compatible assignment.
- Geometry and controller tests must verify ClubGG top/bottom column order `(1/2)`, `(3/4)`, `(5/6)`, odd lower gaps, full-height LDPlayer columns that never become narrower than 9:16, retention of wider detected LDPlayer ratios, repeated-reflow stability, shared-height shrinking, and non-overlap.
- Controller tests must cover identical poker titles/signatures and prove each repeated occurrence resolves to a distinct live window, mirror slot, arrangement request, and non-overlapping rectangle.
- Unit tests must keep the outer shell identity free of `ClubGG` so third-party poker hooks do not target the arranger.
- Unit tests must verify the shared icon retains its blue field, spade silhouette, and centered white plus.

# Work Guidance

- Prefer deterministic fake clocks and mock window backends for debounce tests.
- Property-test arbitrary valid monitor sizes and mixed ClubGG/LDPlayer column sets for containment, aspect preservation, stable order, and maximal shared height.
- Keep failure output free of full external window titles.

# Verification

- `cargo test --all-targets`
- Run the manual checklist in `tests/MANUAL_CLUBGG.md` only with the user present.

# Child DOX Index

- No child documents.
