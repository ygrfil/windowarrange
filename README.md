# ClubGG Table Arranger

A Windows-only companion that arranges independently movable ClubGG poker tables and user-selected application windows. It changes window geometry only; it does not read cards, automate input, or interact with application controls.

## Everyday use

1. Start ClubGG and open one or more poker tables.
2. Run `Table-Arranger-Control.exe` and approve the Windows administrator prompt.
3. Review every discovered normal application window in the single compact list.
4. For ClubGG windows, choose **Arrange** to include a table in the equal-size grid, **Park** to shrink it at the bottom-right, or **Ignore** to leave it untouched.
5. Other application windows default to **Ignore**. Choose **Top-right** to preserve their size at the display corner, or **Fill space** to resize them into the vertical strip to the right of active poker tables.
6. Click one managed poker table number, then another, to swap their positions. Click the selected number again to cancel, use **◎** to locate a window, or **Arrange** for an immediate reflow.
7. Minimize the panel to the system tray when desired.

The compact horizontal panel has a left control rail, click-selectable poker-table tiles, and a separate ordinary-window area. Click one managed poker table number and then another to swap their display positions immediately. The new table order is remembered.

The panel is intentionally small and cannot be maximized. Active poker tables and inactive ClubGG windows are separated into compact groups, while dense two-column grids keep the complete workspace visible without scrolling.

All window choices are remembered across restarts. Ordinary-window rules use both the identified window title and a stable application fallback, so the behavior still applies when a browser tab or document title changes. Selected windows appear first, parked tables next, and ignored windows last within their areas.

**Refresh** discovers windows and immediately reapplies every selected placement even when Auto is off. Turning **Auto** on also immediately applies both poker-table and ordinary-window placement.

**Fill space** never changes the poker layout and never places an application below poker tables. It uses the selected monitor's full-height strip from the rightmost active poker edge to the display's right edge. If several application windows use Fill space, they share the same rectangle and can be switched normally.

One to three active tables use the same maximum table size as the four-table layout. On a 2×2 display layout, two tables occupy the top row and three add the bottom-left position. Four or more tables retain optimized equal-size layouts.

When more than four tables are active, slots 1–4 keep the original 2×2 positions. Additional tables extend to the right in vertical pairs: table 5 above 6, then table 7 above 8.

Default shortcuts:

- `Ctrl+Shift+A` — arrange now.
- `Ctrl+Shift+T` — enable or disable the focused ClubGG table.
- `Ctrl+Shift+P` — show or hide the panel.
- `Ctrl+Shift+F1` through `F8` — toggle numbered tables.

Release builds always request administrator privileges because Windows otherwise blocks movement of the target ClubGG windows. Debug and automated-test artifacts remain non-elevated so verification can run unattended.

The outer process and panel are deliberately named **Table Arranger Control**. Some poker hand-converter software injects ClubGG-specific hooks into any process whose executable or window identity contains `ClubGG`; the neutral shell identity prevents the arranger from being mistaken for the poker client.

Runtime diagnostics are written to:

```text
%APPDATA%\ClubGGTools\ClubGG Table Arranger\config\logs\table-arranger.log
```

## Build

The repository pins Rust 1.97.1 and the exact dependency graph in `Cargo.lock`.

```powershell
cargo build --release --locked
```

Cargo produces the intermediate executable at `target\release\table-arranger-control.exe`. Verified releases are published under `dist\` as both `Table-Arranger-Control.exe` and a matching versioned archive.

## Development

Read the root and applicable child `AGENTS.md` documents before editing. Required checks are:

```powershell
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
cargo tree -d
cargo audit
cargo build --release --locked
```
