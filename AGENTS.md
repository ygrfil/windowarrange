# DOX framework

- DOX is the binding `AGENTS.md` hierarchy installed in this repository.
- This contract is adapted from `agent0ai/dox` revision `5cb5ba55bd1c0f7c1b31fe655fe36e2febb760d2`.
- Upstream: https://github.com/agent0ai/dox

## Core Contract

- `AGENTS.md` files are binding work contracts for their subtrees.
- Work products, source materials, instructions, records, assets, and durable docs must stay understandable from the nearest applicable `AGENTS.md` plus every parent `AGENTS.md` above it.

## Read Before Editing

1. Read the root `AGENTS.md`.
2. Identify every file or folder expected to change.
3. Walk from the repository root to each target path.
4. Read every `AGENTS.md` found along each route.
5. If a parent lists a child `AGENTS.md` whose scope contains the path, read that child and continue from there.
6. Use the nearest `AGENTS.md` as the local contract and parent documents for repository-wide rules.
7. If documents conflict, the closer document controls local details, but no child may weaken DOX.

Do not rely on memory. Re-read the applicable DOX chain in the current session before editing.

## Update After Editing

Every meaningful change requires a DOX pass before the task is complete.

Update the closest owning `AGENTS.md` when a change affects:

- Purpose, scope, ownership, or responsibilities.
- Durable structure, contracts, workflows, or operating rules.
- Required inputs, outputs, permissions, constraints, side effects, or artifacts.
- User preferences about behavior, communication, process, organization, or quality.
- `AGENTS.md` creation, deletion, movement, renaming, or index contents.

Update parent documents when parent-level structure, ownership, workflow, or child indexes change. Update child documents when parent changes alter local rules. Remove stale or contradictory text immediately. Small edits that do not change behavior or contracts may leave documentation unchanged, but the DOX pass still must happen.

## Hierarchy

- The root `AGENTS.md` is the project-wide contract and top-level child index.
- Child `AGENTS.md` files own domain-specific instructions and their own child indexes.
- Each parent explains what its direct children cover and what remains owned by the parent.
- The closer a document is to the work, the more specific and practical it must be.

## Child Document Shape

Create a child `AGENTS.md` when a folder becomes a durable boundary with its own purpose, rules, responsibilities, workflow, materials, or quality standards.

Default section order:

- Purpose
- Ownership
- Local Contracts
- Work Guidance
- Verification
- Child DOX Index

## Style

- Keep documentation concise, current, and operational.
- Document stable contracts, not diary entries.
- Put broad rules in parent documents and concrete details in child documents.
- Prefer direct bullets with explicit names.
- Do not duplicate rules across many files unless each scope needs a local version.
- Delete stale notes instead of explaining history.

## Project Contract

- Build a Windows-only Rust 2024 desktop application named **Table Arranger Control**. Do not provide, retain, or package macOS or Linux runtime support; application dependencies must be Windows-targeted.
- Prefer safe Rust. Confine Win32 `unsafe` operations to the native backend and expose safe typed interfaces.
- Manage only user-approved window geometry. Never automate poker actions, read cards, click controls, or alter application internals.
- Preserve table focus and Z-order while arranging. The explicit user-clicked **Locate** action is the only exception: restore, raise, and request foreground focus for that selected window. Never surprise-move real ClubGG windows from automated tests.
- Anchor poker layouts at the selected monitor's top-left working area. Keep ClubGG tables equal and 4:3 in two-high columns `(1/2)`, `(3/4)`, `(5/6)`; keep each LDPlayer/Pokerrr 2 window at its detected aspect ratio in a separate full-height column. Use full work-area height when the mixed columns fit and reduce their shared height only when required to avoid overlap.
- Support stable ordering, automatic reflow, enabled/parked tables, configurable hotkeys, a floating panel, and tray operation.
- Discover normal visible top-level application windows while excluding the arranger, desktop/taskbar shell surfaces, cloaked windows, child windows, disabled windows, tool windows, and untitled or exact-title `ClubGG` shell surfaces. Treat eligible `dnplayer.exe` main windows as LDPlayer poker tables while excluding LDPlayer headless and service processes.
- Show selected poker and application windows first, then parked poker tables, then ignored windows, with stable session order inside each group. Screen movement must never reorder rows within a group, and signature-based choices must survive restarts.
- Default likely ClubGG and LDPlayer poker tables to **Arrange**. The configurable ordinary-window default starts at **Ignore** and may be changed in Settings to **Top-right** or **Fill space**; an explicit saved window rule always overrides the default.
- **Top-right** preserves an ordinary window's current size and anchors it to the selected display's top-right. **Fill space** resizes it only into the full-height vertical strip between the rightmost active poker-table edge and the selected display's right edge; it must never use space below poker tables. Multiple selected ordinary windows share that strip.
- Provide a persistent, always-clickable **Preserve table slots** user preference in Settings, enabled by default. Anonymous reserved space has a maximum two-column total footprint, not two extra columns; LDPlayer must consume an available reserved column. Apply preservation while fewer than two active columns are occupied, temporarily suppress it at two or more, and restore it below two. A manual Off preference remains off.
- Persist ordinary-window choices with an exact title-aware rule plus a process/class fallback so choices survive title changes and restarts. Exact rules allow different currently identified windows from one application to retain different choices.
- The single **Arrange** action must reconcile discovery and immediately reapply all selected poker and ordinary-window placement. Enabling Auto must also immediately apply the complete workspace.
- Park disabled poker tables at their actual minimum supported sizes, starting at the selected display's bottom-right and continuing directly left without overlap; wrap upward only when a row cannot fit.
- Present a compact workspace board: a single top toolbar with Auto, Arrange, Settings, and tray hiding; a scaled spatial mirror of poker slots and actual parked positions; a separate ordinary-window area; and compact painter-drawn action icons shared by poker and ordinary-window tiles. Actual parked miniatures show the table name, locate on left-click, and unpark on right-click. Put display selection, Preserve table slots, the complete icon legend, default ordinary-window behavior, counts/status, and hotkey details in an independent native Settings window.
- Keep the panel responsive within 680–820 points wide and automatically fit its height between 180 and 420 points to the visible cards. Disable maximizing, fill the full viewport background, and show the complete main workspace without scroll areas. Use dense two-column grids and constrain every tile to its pane.
- Keep Settings independent from the main panel, non-resizable, and automatically fitted to its collapsed or expanded content without scroll areas. Use a deferred native viewport so it can repaint independently without forcing the main panel to stay tall or continuously redraw.
- Keep the compact controller resource-efficient: use the eframe Glow renderer, wake the UI from actual state and input events, and bound/coalesce background event delivery.
- Clicking one managed poker table selects it; clicking another occupied slot swaps individual same-client tables or whole ClubGG/LDPlayer columns, while clicking an empty placeholder moves the selected table and preserves the old hole. Persist spatial assignments and cancel selection when the same table is clicked again.
- Support ClubGG and LDPlayer/Pokerrr 2 profiles and keep discovery profiles extensible for future GGPoker support.
- Keep the executable, PE metadata, app ID, top-level panel title, and tray identity neutral: use **Table Arranger Control** rather than `ClubGG`. Third-party hand converters may otherwise mistake the arranger for the poker client and inject incompatible ClubGG hooks.
- Use one shared app-icon definition for the executable, window, and tray: a royal-blue field with a dark-blue spade silhouette and a centered white plus.
- Deliver manually launched, portable `x86_64-pc-windows-msvc` release executables under `dist/` without an installer or startup registration. Keep `Table-Arranger-Control.exe` as the current release and a matching `Table-Arranger-Control-<version>.exe` archive.

## Dependency Policy

- Pin the stable Rust toolchain in `rust-toolchain.toml` with `rustfmt` and `clippy`.
- The verified release baseline is Rust/Cargo `1.97.1` with `windows 0.62.2`, `eframe 0.35.0`, `tray-icon 0.24.2`, `global-hotkey 0.8.0`, `serde 1.0.229`, `serde_json 1.0.151`, `thiserror 2.0.19`, `log 0.4.33`, `crossbeam-channel 0.5.16`, and development dependency `proptest 1.11.0`.
- Use current stable crates from crates.io only; do not use prereleases, wildcards, or unpinned Git dependencies.
- Disable default features when practical and enable only required Windows functionality.
- Commit `Cargo.lock` and treat it as the exact reproducible dependency graph.
- If the newest stable release is incompatible, use the newest verified stable release and record the reason here.
- Duplicated transitive dependency families introduced by eframe/Glutin, Windows support crates, and development-only property testing are expected and reviewed with `cargo tree -d`; do not force-deduplicate incompatible semantic versions.

## Repository Workflow

- Keep application code under `src/`, integration and property tests under `tests/`, and Windows resources under `assets/`.
- Publish release artifacts only under `dist/`. Before publishing, replace the stale current executable, then copy the verified build as both the current neutral filename and the versioned archive. Preserve older versioned archives unless the user explicitly approves their deletion.
- Keep full poker-window titles out of ordinary logs and persisted diagnostics.
- Ship the release executable with a `requireAdministrator` manifest because ClubGG table movement on the target system requires matching elevated privileges. Windows must display the standard UAC prompt at launch.
- Keep debug and test artifacts `asInvoker` so automated verification can execute without elevation; this exception does not apply to delivered release binaries.
- Write redacted runtime diagnostics to `%APPDATA%\ClubGGTools\ClubGG Table Arranger\config\logs\table-arranger.log`.
- Keep live ClubGG and LDPlayer verification explicit and manual.

## Verification

Before delivery, run:

- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all-targets`
- `cargo tree -d`
- `cargo audit`
- `cargo build --release --locked`

## Closeout

1. Re-check changed paths against the DOX chain.
2. Update nearest owning documents and any affected parents or children.
3. Refresh every affected child index.
4. Remove stale or contradictory text.
5. Run applicable verification.
6. Report documentation intentionally left unchanged and why.

## User Preferences

- Use Rust where it is suitable; it is the selected implementation language.
- Keep the application strictly Windows-only; do not spend dependencies or implementation effort on macOS or Linux support.
- Use automatic arrangement plus manual hotkeys.
- Keep spatial assignments stable while the saved Preserve table slots preference is on. Closing or ignoring releases identity ownership to an anonymous hole; Park retains an owned ghost. Never let anonymous preservation expand the layout beyond two total columns, let LDPlayer claim a reserved column, and omit anonymous reserved columns at two or more active columns; manual Off compacts persistently.
- Preserve the ClubGG 4:3 shape and each LDPlayer window's detected Pokerrr 2 aspect ratio.
- Float the compact panel above tables and allow it to minimize to the system tray.
- Add new tables as enabled at the end of the current order.
- Do not immediately undo manual moves; wait until the next structural reflow or explicit arrange action.
- Leave disabled tables parked when the arranger exits.
- Launch manually and deliver a portable executable only.
- Keep every delivered release in `dist/`, replacing the stale current executable before publishing while retaining versioned archives by default.
- Preserve the neutral Table Arranger Control shell identity for compatibility with Asian Hand Converter and similar software.
- Keep the executable, window, and tray icon blue with a spade silhouette and a centered white plus.
- Launch release builds as administrator through the embedded manifest.
- Keep the panel compact and modern: a short horizontal toolbar, a scaled desktop-proportional poker mirror with labeled placeholders, parked ghosts, and interactive named parked miniatures; a separate ordinary-window area with matching vector controls; and all secondary controls/status in Settings.
- Open Settings as a separate compact native window that remains fully visible when its hotkey section is expanded and does not inflate or obscure the main workspace.
- Keep the workspace visually structured and space-efficient: start poker and ordinary-window cards directly below the toolbar, omit section descriptions, separate active poker tables from parked and ignored poker windows with spacing and a divider, and automatically shrink panel height when fewer rows are visible without clipping the final row.
- Number ClubGG geometry down each column: 1 above 2, 3 above 4, and 5 above 6. Leave the lower half of an odd final ClubGG column empty.
- Place LDPlayer columns after ClubGG by default. A manual mixed swap exchanges one LDPlayer column with the selected ClubGG pair or single-plus-empty column.
- Discover ordinary open application windows and default them to Ignore. Let the user change the global default to Ignore, Top-right, or Fill-space in Settings, while persistent per-window choices override it and ordinary windows never join the poker grid.
- Keep Fill-space application windows strictly to the right of active poker tables for every table count, including one or two; never choose a larger empty band underneath the tables.
- Preserve up to a two-column total poker footprint for Fill-space applications and mirrored Placeholders by default while fewer than two active columns are occupied; LDPlayer consumes that footprint, preservation is inactive at two or more active columns, and manual Off plus spatial assignments survive restart.
- Remember ordinary-window behavior across restarts even when its title changes, and remember poker column/slot assignments after click-to-swap or placeholder moves.
- Keep every primary toolbar action visible without overlap and avoid decorative glyphs that are missing from the bundled font.
- Make Locate restore and raise the chosen window so it is visible above windows that were covering it; requesting foreground focus is expected for this explicit action.
- Keep idle RAM, GPU-memory, and CPU use proportionate to a compact control utility without weakening window-event responsiveness.
- Park disabled tables at minimum size shoulder-to-shoulder from the bottom-right toward the left instead of overlapping them.
- Debounce native window-event discovery by 200 milliseconds while keeping explicit Arrange, Auto, settings changes, and hotkey actions immediate.

## Child DOX Index

- `src/AGENTS.md` — application runtime, native Windows integration, controller, configuration, and UI.
- `tests/AGENTS.md` — property, integration, synthetic-window, and manual ClubGG verification.
