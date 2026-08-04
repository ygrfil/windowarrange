# Purpose

- Own the ClubGG Table Arranger runtime and all code shipped in the portable executable.

# Ownership

- `lib.rs` defines the reusable module boundary shared by the executable and integration tests.
- `win32.rs` owns native enumeration, process inspection, monitor APIs, movement, explicit Locate activation, DPI, and integrity errors.
- `layout.rs` owns pure, deterministic mixed ClubGG/LDPlayer column geometry and right-side free-strip calculation.
- `model.rs` owns shared typed state.
- `controller.rs` owns discovery reconciliation, ordering, enabled state, debounce, and arrangement.
- `config.rs` owns versioned persistence.
- `identity.rs` owns the neutral outer process and panel identity.
- `logging.rs` owns bounded persistent file diagnostics through the lightweight `log` facade; do not add a runtime subscriber or filtering framework without a measured need.
- `hotkeys.rs` and `tray.rs` own their respective Windows integrations.
- `app.rs` owns the eframe/egui panel.
- `main.rs` owns startup, single-instance behavior, logging, and dependency wiring.

# Local Contracts

- Confine `unsafe` blocks to native integration modules and document each safety boundary.
- Never activate or reorder target windows while arranging.
- Locate is the deliberate exception to arrangement focus safety: restore a minimized target, raise it in Z-order, request foreground focus, and flash only as a fallback when Windows denies foreground activation.
- Never persist transient `HWND` values across processes or launches.
- Keep the controller as the sole mutable owner of managed-table order and enabled state.
- Keep layout calculation independent of Win32, eframe, and global state.
- Redact full poker-window titles from logs.
- Apply the Windows legacy extension-point mitigation before initializing the GUI, hooks, hotkeys, or tray.
- Do not put `ClubGG` back into the executable name, PE metadata, app ID, top-level panel title, or tray identity; ClubGG-specific labels may remain inside the control panel.
- Release resources require administrator startup; build-script resources keep debug and test artifacts `asInvoker`.
- `assets/icon.rs` is the single pixel source for the executable, window, and tray icon. Keep its royal-blue field, dark-blue spade silhouette, and centered white plus consistent across all surfaces.
- Preserve candidate session order regardless of window coordinates. Managed poker order comes from persisted spatial columns; ignored poker and ordinary-window rows retain stable session order.
- The main panel uses a top-aligned workspace board. Its single toolbar owns Auto, Arrange, Settings, and tray hiding; the poker pane is a uniformly scaled mirror of the selected work area with active slots, muted numbered Placeholders tiles, parked ghosts, and named interactive parked miniatures; the ordinary-window board owns compact application cards. An independent native Settings viewport owns display selection, effective Preserve table slots state, the icon legend, the ordinary-window default, counts/status, and hotkey details.
- The top toolbar must remain content-height and never consume unnecessary workspace height. The workspace begins immediately below it, without a title, descriptions, or summary header.
- The main viewport is non-maximizable, 680–820 points wide, and automatically fits between 180 and 420 points high according to visible card rows, with a 740×260 initial size. Height calculations must include toolbar, separator, frame margins, row gaps, and group dividers so the final card row is fully visible. The root frame fills all available space. The main workspace has no scroll areas: poker groups use dense two-column rows, ordinary windows use compact rows, and no child grid/card may expand its parent pane.
- Settings uses eframe's deferred native viewport with the shared application icon and snapshot state. It is non-resizable, independently repainted, and automatically fitted between 240 and 700 points high as its hotkey section collapses or expands; it must not use a scroll area or alter the main viewport height.
- Project poker rectangles with one uniform monitor-to-pane scale and center the resulting work-area mirror. Keep ordinary-window cards and ignored-poker rows within the same viewport fitting model, including bottom-frame allowance for DPI scaling and native size rounding.
- Draw Locate, Active, Park, Ignore, Fill space, and Top-right controls with egui painter primitives rather than font glyphs. Poker and ordinary-window cards share this icon language. Keep full action names in tooltips and a short complete legend in Settings.
- UI geometry is persisted in `ui-state-v4.ron`; the v4 filename discards stale taller panel geometry while preserving window-management choices in the main configuration.
- ClubGG and LDPlayer tiles expose icon controls for **Locate**, **Active**, **Park**, and **Ignore**. Ordinary application cards expose painter-drawn **Locate**, **Ignore**, **Fill space**, and **Top-right** controls. A real parked miniature locates on primary click and unparks on secondary click without changing the ghost's slot contract.
- Configuration schema version 7 persists table, parked, ignored, top-right, and free-space dispositions; typed poker columns and holes; the default-on Preserve table slots behavior; legacy order for migration; and the configurable ordinary-window default. Missing fields deserialize safely, and the version-6 `reserve_two_slots` key aliases to the new setting. Ordinary and LDPlayer choices write exact plus process/class fallback rules; exact matches take priority.
- The single Arrange command reconciles immediately and then arranges regardless of Auto state. Turning Auto on also reconciles and arranges immediately. Native location events retain debounce and manual-movement protections.
- Use native window events as the primary discovery trigger with a 200-millisecond trailing debounce and a ten-second fallback reconciliation. Subscribe only to lifecycle and location ranges, and ignore child-window location events before queueing work. Explicit Arrange and enabling Auto remain immediate.
- Use bounded background channels. The controller snapshot channel shares immutable snapshots, retains only the newest state, suppresses unchanged snapshots, and explicitly wakes egui when a new state is published; do not add a periodic UI repaint loop.
- Keep the controller and native-event thread stacks explicitly bounded because their workloads are shallow and heap-backed.
- Use eframe's Glow renderer for the compact panel unless measurements and compatibility testing justify a different backend.
- Poker clicks use a two-step selection. A second occupied slot emits a typed source-window/destination-slot command; an empty placeholder is also a valid destination. The controller performs individual same-client swaps or whole mixed-client column swaps, persists assignments, and immediately arranges; UI code never moves windows directly.
- `layout.rs::calculate_mixed_layout` gives each ClubGG column two equal 4:3 cells and each LDPlayer column one full-height detected-aspect cell. It maximizes a shared even column height within the work area, preserving left-to-right unit order and avoiding overlap.
- `layout.rs::right_side_free_rect` returns the full-height monitor strip starting at the rightmost active poker-table edge. Fill-space application windows may overlap one another, must never use empty space below poker tables, and do not treat parked-table bounds as occupied.
- Treat the persisted Preserve table slots value as an always-editable preference and derive an effective value. Anonymous reserved geometry is capped at two total columns; new or previously appended LDPlayer assignments claim an available reserved column. At two or more active columns, omit anonymous reserved columns without overwriting the preference; below two, restore only enough anonymous geometry to reach two total columns. Manual Off removes holes, compacts, persists, and immediately calls Arrange regardless of Auto state.
- Park every disabled ClubGG or LDPlayer table at its queried minimum size using its own preferred aspect ratio. Lay parked tables out in stable table order from the selected work area's bottom-right toward the left with no overlap or gap, wrapping upward only when necessary.
- Do not manipulate internal ClubGG or LDPlayer child controls; only independently movable eligible windows are managed. Exclude untitled and exact-title `ClubGG` shell surfaces so they do not appear as real table tiles. Match LDPlayer only through the `dnplayer.exe` frontend, never its headless or service processes.
- General discovery includes visible, non-cloaked, titled, non-tool top-level application windows and excludes the current process and Windows desktop/taskbar surfaces. The ordinary-window default is configurable as Ignore, Fill space, or Top-right and starts at Ignore; exact or fallback saved rules override it.
- UI actions send typed commands; the UI must not call Win32 table operations directly.
- The crate must fail compilation for non-Windows targets, and application dependencies belong under the Cargo Windows target table. Do not add macOS, Linux, web, X11, or Wayland runtime features.

# Work Guidance

- Prefer explicit error/status values over panics in runtime paths.
- Treat destroyed handles and access denial as recoverable per-window conditions.
- Suppress self-generated location events so arranging does not create an event loop.
- Keep the mirrored placeholders and UI usable when no poker process or table is present.
- Use bundled-font-safe text for always-visible toolbar and table-selection controls; tooltips may clarify compact labels.
- Preserve current size for Top-right placement unless it exceeds the selected working area. Refuse Fill-space movement when the remaining rectangle is smaller than the target window's minimum size.

# Verification

- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all-targets`
- `cargo build --release --locked`

# Child DOX Index

- No child documents. Add one only when a source subdirectory becomes a durable domain boundary.
