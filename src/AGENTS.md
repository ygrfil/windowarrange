# Purpose

- Own the ClubGG Table Arranger runtime and all code shipped in the portable executable.

# Ownership

- `lib.rs` defines the reusable module boundary shared by the executable and integration tests.
- `win32.rs` owns native enumeration, process inspection, monitor APIs, movement, highlighting, DPI, and integrity errors.
- `layout.rs` owns pure, deterministic table geometry and largest-free-rectangle calculation.
- `model.rs` owns shared typed state.
- `controller.rs` owns discovery reconciliation, ordering, enabled state, debounce, and arrangement.
- `config.rs` owns versioned persistence.
- `identity.rs` owns the neutral outer process and panel identity.
- `logging.rs` owns bounded persistent file diagnostics.
- `hotkeys.rs` and `tray.rs` own their respective Windows integrations.
- `app.rs` owns the eframe/egui panel.
- `main.rs` owns startup, single-instance behavior, logging, and dependency wiring.

# Local Contracts

- Confine `unsafe` blocks to native integration modules and document each safety boundary.
- Never activate or reorder target windows while arranging.
- Never persist transient `HWND` values across processes or launches.
- Keep the controller as the sole mutable owner of managed-table order and enabled state.
- Keep layout calculation independent of Win32, eframe, and global state.
- Redact full ClubGG titles from logs.
- Apply the Windows legacy extension-point mitigation before initializing the GUI, hooks, hotkeys, or tray.
- Do not put `ClubGG` back into the executable name, PE metadata, app ID, top-level panel title, or tray identity; ClubGG-specific labels may remain inside the control panel.
- Release resources require administrator startup; build-script resources keep debug and test artifacts `asInvoker`.
- Preserve candidate session order regardless of window coordinates. The UI groups selected windows, parked tables, and ignored windows in that order while preserving session order inside each group.
- The main panel uses a top-aligned horizontal workspace board. Its left rail owns Auto, Arrange, Refresh, and Settings; the poker board separates active and inactive two-column groups with a thin divider but no section labels; the ordinary-window board owns compact unlabeled application cards. The first cards align vertically with Auto. Display selection, status, Locate, tray hiding, and settings remain available.
- The top bar must remain content-height and never consume the workspace height. Apply top alignment to the workspace columns, not to the header row containing the right-to-left display controls.
- The main viewport is non-maximizable and bounded to 680×350 through 820×460, with a 740×380 default. The root frame fills all available space. The main workspace has no scroll areas: poker groups use dense two-column 44–62 point rows, ordinary windows use 48–62 point rows, and no child grid/card may expand its parent pane.
- Two-column poker rows must allocate each tile with an explicit fixed-size top-down layout. Never let a tile inherit its row's horizontal layout, which causes each child to expand to the full pane width.
- UI geometry is persisted in `ui-state-v3.ron`; the v3 filename intentionally discards the larger geometry saved through 0.4.1 while preserving window-management choices in the main configuration.
- ClubGG tiles expose **Active**, **Park**, and **Ignore**. Ordinary application cards expose **Ignore**, **Fill space**, and **Top-right**.
- Configuration schema version 4 persists table, parked, ignored, top-right, and free-space dispositions plus table order. Ordinary-window choices write an exact signature and a process/class fallback; exact matches take priority.
- A Refresh command reconciles immediately and arranges regardless of Auto state. Turning Auto on reconciles and arranges immediately. Native location events retain debounce and manual-movement protections.
- Use native window events as the primary discovery trigger with a 200-millisecond trailing debounce and a ten-second fallback reconciliation. Explicit Refresh and enabling Auto remain immediate.
- Use bounded background channels. The controller snapshot channel retains only the newest state, suppresses unchanged snapshots, and explicitly wakes egui when a new state is published; do not add a periodic UI repaint loop.
- Use eframe's Glow renderer for the compact panel unless measurements and compatibility testing justify a different backend.
- Poker table-number clicks use a two-step selection: the first click selects, a second click on the same number cancels, and a click on another managed table number emits the typed `Reorder` command. The controller persists reordered signatures and immediately arranges; UI code never moves windows directly.
- `layout.rs` evaluates a virtual count of four for 1–3 active tables and returns only the requested first row-major rectangles. Counts 4+ retain maximal-area grid selection. For counts above four, preserve the initial 2×2 slot order `(1,2)/(3,4)`, then extend right in vertical pairs `5/6` and `7/8`; non-two-row grids preserve the same initial 2×2 block before filling remaining cells deterministically.
- `layout.rs::right_side_free_rect` returns the full-height monitor strip starting at the rightmost active poker-table edge. Fill-space application windows may overlap one another, must never use empty space below poker tables, and do not treat parked-table bounds as occupied.
- Do not manipulate internal ClubGG child controls; only independently movable top-level or owned-popup windows are eligible.
- General discovery includes visible, non-cloaked, titled, non-tool top-level application windows and excludes the current process and Windows desktop/taskbar surfaces. Ordinary windows default to ignored.
- UI actions send typed commands; the UI must not call Win32 table operations directly.

# Work Guidance

- Prefer explicit error/status values over panics in runtime paths.
- Treat destroyed handles and access denial as recoverable per-window conditions.
- Suppress self-generated location events so arranging does not create an event loop.
- Keep the UI usable when no ClubGG process or table is present.
- Use bundled-font-safe text for always-visible rail and table-selection controls; tooltips may clarify compact labels.
- Preserve current size for Top-right placement unless it exceeds the selected working area. Refuse Fill-space movement when the remaining rectangle is smaller than the target window's minimum size.

# Verification

- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all-targets`
- `cargo build --release --locked`

# Child DOX Index

- No child documents. Add one only when a source subdirectory becomes a durable domain boundary.
