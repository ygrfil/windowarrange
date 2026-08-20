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
- `rng_overlay.rs` owns the optional native transparent 1–100 RnG overlay window, its worker lifecycle, and its migrated persistent settings.
- `hotkeys.rs` and `tray.rs` own their respective Windows integrations.
- `app.rs` owns the eframe/egui panel.
- `main.rs` owns startup, single-instance behavior, logging, and dependency wiring.

# Local Contracts

- Confine `unsafe` blocks to native integration modules and document each safety boundary.
- Never activate or reorder target windows while arranging.
- Locate is the deliberate exception to arrangement focus safety: restore a minimized target, raise it and its owned-window chain with ordinary Z-order APIs, request foreground focus, and flash only as a fallback when Windows denies foreground activation. Keep this path simple; do not add persistent or temporary topmost behavior.
- Never persist transient `HWND` values across processes or launches.
- Keep the controller as the sole mutable owner of managed-table order and enabled state.
- Keep layout calculation independent of Win32, eframe, and global state.
- Redact full poker-window titles from logs.
- Apply the Windows legacy extension-point mitigation before initializing the GUI, hooks, hotkeys, or tray.
- Do not put `ClubGG` back into the executable name, PE metadata, app ID, top-level panel title, or tray identity; ClubGG-specific labels may remain inside the control panel.
- Release resources require administrator startup; build-script resources keep debug and test artifacts `asInvoker`.
- `assets/icon.rs` is the single pixel source for the executable, window, and tray icon. Keep its royal-blue field, dark-blue spade silhouette, and centered white plus consistent across all surfaces.
- Preserve candidate session order regardless of window coordinates. Managed poker order comes from persisted spatial columns; ignored poker and ordinary-window rows retain stable session order.
- The main panel uses a top-aligned unified workspace board. Its single toolbar owns Auto, the adjacent **RnG** overlay toggle, Arrange, the **Space** toggle for Preserve table slots, Settings, and tray hiding, with Space between Arrange and Settings. One full-width uniformly scaled mirror of the selected work area shows active slots, muted numbered Placeholders tiles, named interactive parked-table miniatures at their actual positions, and no reserved parked-table ghosts. A subtle divider attaches a responsive six-to-eight-column dock containing the combined lobby-count chip first, mode-sorted ordinary-window chips next, and ignored-poker chips last. An independent native Settings viewport owns display selection, the icon legend, the ordinary-window default, counts/status, and hotkey details.
- The top toolbar must remain content-height and never consume unnecessary workspace height. The workspace begins immediately below it, without a title, descriptions, or summary header.
- The main viewport is non-maximizable, 680–820 points wide, and automatically fits between 180 and 620 points high from its actual width, selected-work-area aspect ratio, and visible dock rows, with a 740×480 initial size. Height calculations must include toolbar, separators, frame margins, and row gaps so the final dock row is fully visible. The root frame fills all available space. The main workspace has no scroll areas, and no dock chip may expand its fixed cell.
- Settings uses eframe's deferred native viewport with the shared application icon and snapshot state. It is non-resizable, independently repainted, and automatically fitted between 240 and 700 points high as its hotkey section collapses or expands; it must not use a scroll area or alter the main viewport height.
- Project poker rectangles with one uniform monitor-to-board scale and center the resulting full-width work-area mirror. Keep ordinary-window, lobby, and ignored-poker chips within the attached responsive dock and the same viewport fitting model, including bottom-frame allowance for DPI scaling and native size rounding.
- Draw Active, Park, Ignore, Fill space, and Top-right controls with egui painter primitives rather than font glyphs. Poker tiles use Active/Park/Ignore and ordinary-window dock chips use Ignore/Fill space/Top-right. For poker tiles, ignored-poker chips, and ordinary-window chips, subtract only the exact action controls and poker number badge from the full visual bounds; every remaining pixel performs Locate. The combined lobby chip uses its direct Locate button. Keep full action names in tooltips and a short complete legend in Settings.
- UI geometry is persisted in `ui-state-v4.ron`; the v4 filename discards stale taller panel geometry while preserving window-management choices in the main configuration.
- ClubGG and LDPlayer tiles expose icon controls for **Active**, **Park**, and **Ignore**; primary-clicking the tile body performs Locate, while only its enlarged numbered-badge hit target participates in swap selection. Ordinary application dock chips expose **Ignore**, **Fill space**, and **Top-right** controls and locate on body click. The combined lobby chip uses a direct button response and invokes the ordinary single-window Locate path for each lobby sequentially with a short gap, raising each lobby at its existing parked position without resizing, repositioning, or changing its saved Park disposition. Parking releases the poker slot and compacts active assignments; a real parked miniature locates on primary click and unparks on secondary click as a newly active assignment.
- Configuration schema version 8 persists table, parked, ignored, top-right, and free-space dispositions; typed poker columns; explicit anonymous closed-table placeholders; the default-on Preserve table slots behavior; legacy order for migration; and the configurable ordinary-window default. Missing fields deserialize safely, and the version-6 `reserve_two_slots` key aliases to the new setting. Ordinary and LDPlayer choices write exact plus process/class fallback rules; exact matches take priority.
- The single Arrange command reconciles immediately and then arranges regardless of Auto state. Turning Auto on also reconciles and arranges immediately. Native location events retain debounce and manual-movement protections.
- The configured panel hotkey toggles the main panel: it hides a currently visible panel and restores, shows, and focuses a hidden or minimized panel. Tray activation remains a one-way Show action.
- Use native window events as the primary discovery trigger with a 200-millisecond trailing debounce and a ten-second fallback reconciliation. Subscribe only to lifecycle and location ranges, and ignore child-window location events before queueing work. Explicit Arrange and enabling Auto remain immediate.
- On a debounced native location event, map a moved active table's center into the compatible mixed-layout slot beneath it, update/swap persisted ownership, and refresh the mirror without forcing an immediate arrangement. Do not infer screen order during startup, fallback discovery, or explicit Arrange reconciliation.
- Use bounded background channels. The controller snapshot channel shares immutable snapshots, retains only the newest state, suppresses unchanged snapshots, and explicitly wakes egui when a new state is published; do not add a periodic UI repaint loop.
- Keep the controller and native-event thread stacks explicitly bounded because their workloads are shallow and heap-backed.
- Use eframe's Glow renderer for the compact panel unless measurements and compatibility testing justify a different backend.
- RnG is an in-process native Win32 layered window on a bounded worker thread, never an eframe child viewport or separately packaged executable. It starts Off, follows the selected monitor work area, remains always-on-top without a taskbar entry or decorations, rerolls from 1–100 on its interval or primary click, and provides right-click interval/color/size/corner controls. Reuse `%APPDATA%\SunnyRandomiser\settings.txt` so choices from the original standalone utility migrate and future changes persist.
- Non-control card-body clicks send Locate for poker tables and ordinary windows; the combined lobby button sends a group command that calls the same single-window Locate backend sequentially for each lobby without issuing any geometry change and while preserving Park mode. Non-overlapping enlarged number-badge clicks use a two-step swap selection: a second occupied number emits a typed source-window/destination-slot command, and an empty placeholder is also a valid destination. The controller performs individual same-client swaps or whole mixed-client column swaps, persists assignments, and immediately arranges; UI code never moves windows directly.
- Resolve persisted poker signatures as an ordered multiset. Multiple live tables may legitimately share the same title/signature; assign repeated occurrences to distinct unused window IDs and use those resolved IDs for arrangement, slot release, swaps, mirror occupants, and active-column calculations.
- `layout.rs::calculate_mixed_layout` gives each ClubGG column two equal 4:3 cells and each LDPlayer column one full-height aspect-preserving cell. Normalize LDPlayer input to at least the natural Pokerrr 2 ratio of 9:16 while retaining wider detected ratios, then maximize a shared even column height within the work area without overlap.
- `layout.rs::right_side_free_rect` returns the full-height monitor strip starting at the rightmost active poker-table edge. Calculate poker geometry first, then force every Fill-space application into that exact remaining strip even when it is narrower than the application's reported minimum size; Fill-space applications may overlap one another, must never overlap poker or use empty space below it, and do not treat parked-table bounds as occupied.
- Treat the persisted Preserve table slots value as an always-editable preference and derive an effective value. Baseline anonymous reserved geometry is capped at two total columns; explicit placeholders created by closed tables retain their previously occupied slots and may retain a formerly occupied whole column beyond that baseline. New tables consume the earliest compatible anonymous slot through normal assignment, without identity ownership. At two or more active columns, omit unused baseline columns without overwriting the preference. Manual Off removes holes, compacts, persists, and immediately calls Arrange regardless of Auto state.
- Park every disabled ClubGG/LDPlayer table and every parked ClubGG lobby at its queried minimum size using its own preferred aspect ratio. Lay parked poker tables out in stable order from the selected work area's top-right toward the left, wrapping downward. Independently keep parked ClubGG lobbies ordered from the bottom-right toward the left, wrapping upward.
- Do not manipulate internal ClubGG or LDPlayer child controls; only independently movable eligible windows are managed. Classify untitled, exact-title, and explicitly lobby-titled ClubGG top-level surfaces as ancillary **ClubGG lobby** candidates. Default every lobby to Park, combine them into one count card directly beneath the poker mirror with group Park/Ignore/Top-right commands, and never assign them a poker column, half-slot, table number/hotkey, preserved placeholder, or parked ghost. Match LDPlayer only through the `dnplayer.exe` frontend, never its headless or service processes.
- General discovery includes visible, non-cloaked, titled, non-tool top-level application windows and excludes the current process and Windows desktop/taskbar surfaces. The ordinary-window default is configurable as Ignore, Fill space, or Top-right and starts at Ignore; exact or fallback saved rules override it.
- UI actions send typed commands; the UI must not call Win32 table operations directly.
- The crate must fail compilation for non-Windows targets, and application dependencies belong under the Cargo Windows target table. Do not add macOS, Linux, web, X11, or Wayland runtime features.

# Work Guidance

- Prefer explicit error/status values over panics in runtime paths.
- Treat destroyed handles and access denial as recoverable per-window conditions.
- Suppress self-generated location events so arranging does not create an event loop.
- Keep the mirrored placeholders and UI usable when no poker process or table is present.
- Use bundled-font-safe text for always-visible toolbar and table-selection controls; tooltips may clarify compact labels.
- Preserve current size for Top-right placement unless it exceeds the selected working area. For Fill-space, poker has priority: always request the exact remaining right-side rectangle instead of leaving an ordinary window at stale overlapping geometry when the strip is narrower than its reported minimum.

# Verification

- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all-targets`
- `cargo build --release --locked`

# Child DOX Index

- No child documents. Add one only when a source subdirectory becomes a durable domain boundary.
