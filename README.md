# Table Arranger Control

A Windows-only companion that arranges independently movable ClubGG and LDPlayer/Pokerrr 2 poker windows plus user-selected ordinary applications. It changes approved window geometry only; it does not read cards, automate input, or interact with application controls.

## Everyday use

1. Start ClubGG and/or Pokerrr 2 in LDPlayer 9, then open the poker tables you want.
2. Run `Table-Arranger-Control.exe` and approve the Windows administrator prompt.
3. Use the mirrored poker board to confirm positions. ClubGG uses two-high 4:3 columns `(1/2)`, `(3/4)`, and `(5/6)`; each LDPlayer window uses its own full-height aspect-preserving column after ClubGG by default.
4. Use the small vector icons for Locate, Active, Park, Ignore, Fill space, and Top-right. Poker and ordinary-window cards share this icon style; tooltips and the short Settings legend explain every symbol.
5. Select one poker tile and then another occupied slot to swap them. A ClubGG/LDPlayer swap moves the whole ClubGG pair. Select a table and then a dashed empty placeholder to move it while preserving the old hole.
6. Leave **Preserve table slots** enabled to retain holes and parked reservations inside a maximum two-column reserved footprint. LDPlayer consumes an available reserved column instead of being appended after it. Preservation becomes inactive when two or more active columns already occupy the footprint, then returns below two; its preference remains clickable, and a manual Off choice remains off.
7. Parked tables move to their actual minimum sizes along the display's bottom-right. Their active positions remain as orange ghosts while preservation is enabled. On the miniature real parked table, left-click locates it and right-click unparks it.
8. Ordinary windows remain separate and default to Ignore. Choose Top-right to preserve size at the corner or Fill space to occupy only the full-height strip to the right of the poker layout.

The mirrored board scales the selected monitor's working area into the compact panel. Active table tiles, muted numbered **Placeholders** targets, parked ghosts, and named actual parked miniatures retain their desktop-relative positions. ClubGG lobby windows appear separately in the right-side **ClubGG lobbies** group and default to Park. They never consume table slots or preserved space, and multiple parked lobbies line up shoulder-to-shoulder with the parked tables. Ignored poker windows remain accessible in the compact unmanaged row.

All dispositions, spatial slots, manual holes, monitor choice, hotkeys, and ordinary-window rules are remembered. Closing or ignoring a poker window releases its signature from the slot immediately; with preservation enabled, the position remains as an anonymous empty target for another table.

**Arrange** discovers current windows and immediately reapplies every selected placement even when Auto is off. Enabling **Auto** also immediately applies poker and ordinary-window placement. Native discovery events use a 200 ms trailing debounce.

Mixed columns use the monitor's full working height when they fit. LDPlayer keeps at least Pokerrr 2's natural 9:16 portrait ratio and retains any wider detected ratio, so repeated Auto reflows cannot make it progressively narrower. When several columns no longer fit, the layout reduces their shared height while preserving ratios and preventing overlap.

**Fill space** never uses a band below poker tables. It begins at the right edge of the actual or preserved poker-column footprint and extends to the selected monitor's right edge. Multiple Fill-space applications share that rectangle.

Default shortcuts:

- `Ctrl+Shift+A` — arrange now.
- `Ctrl+Shift+T` — enable or park the focused managed poker table.
- `Ctrl+Shift+P` — show or hide the panel.
- `Ctrl+Shift+F1` through `F8` — toggle numbered real poker windows.

Release builds request administrator privileges because Windows otherwise blocks movement of the target poker windows. Debug and automated-test artifacts remain non-elevated. Automated tests never move live ClubGG or LDPlayer windows.

The outer executable, panel, taskbar, and tray identity remain neutral **Table Arranger Control** so hand-converter software does not mistake the arranger for ClubGG. All surfaces use the same blue spade icon with a centered white plus.

Runtime diagnostics are written to:

```text
%APPDATA%\ClubGGTools\ClubGG Table Arranger\config\logs\table-arranger.log
```

## Build

The repository pins Rust 1.97.1 and the exact dependency graph in `Cargo.lock`.

```powershell
cargo build --release --locked
```

Cargo produces `target\release\table-arranger-control.exe`. Verified portable releases are copied to `dist\Table-Arranger-Control.exe` and a matching versioned archive.

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
