# Table Arranger Control

A Windows-only companion that arranges independently movable ClubGG and LDPlayer/Pokerrr 2 poker windows plus user-selected ordinary applications. It changes approved window geometry only; it does not read cards, automate input, or interact with application controls.

## Everyday use

1. Start ClubGG and/or Pokerrr 2 in LDPlayer 9, then open the poker tables you want.
2. Run `Table-Arranger-Control.exe` and approve the Windows administrator prompt.
3. Press **RnG** beside Auto to show the transparent 1–100 number overlay; its selected color communicates that it is On. Press it again to switch it off. Left-click the number to reroll immediately. Right-click it to change interval, color, size, or corner. Those choices persist across launches, while RnG itself starts Off.
4. Use the mirrored poker board to confirm positions. ClubGG uses two-high 4:3 columns `(1/2)`, `(3/4)`, and `(5/6)`; each LDPlayer window uses its own full-height aspect-preserving column after ClubGG by default.
5. Left-click anywhere on a poker-table tile, ignored-poker chip, or ordinary-window card to Locate it; only the exact action buttons and poker numbered badge are excluded. **GGLobby** raises detected lobbies one by one. Click only a poker tile's numbered badge when selecting tables for swapping. The remaining small vector icons control Active/Park/Ignore on poker tables and Ignore/Fill space/Top-right on ordinary windows.
6. Select one poker tile and then another occupied slot to swap them. A ClubGG/LDPlayer swap moves the whole ClubGG pair. Select a table and then a dashed empty placeholder to move it while preserving the old hole.
7. Leave **Preserve table slots** enabled to retain anonymous baseline space inside a maximum two-column footprint plus explicit holes left by closed tables. LDPlayer consumes an available baseline column instead of being appended after it. Preservation becomes inactive when two or more active columns already occupy the footprint, then returns below two; its preference remains clickable, and a manual Off choice remains off.
8. Parked tables move to their actual minimum sizes from the display's top-right toward the left. Each parked table gets its own ClubGG or Pokerrr 2 application-icon button beside **GGLobby**; left-click locates it and right-click unparks it, removing the button.
9. Ordinary windows remain separate and default to Ignore. Choose Top-right to preserve size at the corner or Fill space to force the window into only the full-height strip remaining to the right after poker is laid out.

The mirrored board scales the selected monitor's working area into the compact panel. Active table tiles and muted numbered **Placeholders** targets retain their desktop-relative positions; parked tables stay out of the mirror and use their toolbar buttons instead. ClubGG lobbies default to Park, and **GGLobby** runs the ordinary Locate action for each lobby sequentially, raising it exactly where it is parked without resizing, repositioning, or changing its saved Park setting. Lobbies never consume table slots or preserved space. Ignored poker windows remain accessible in the compact unmanaged row.

All dispositions, spatial slots, manual holes, monitor choice, hotkeys, and ordinary-window rules are remembered. Identical displayed poker names are treated as separate live windows and retain separate slots. Closing or ignoring a poker window releases its assigned occurrence from the slot immediately; with preservation enabled, the position remains as an anonymous empty target for another table.

**Arrange** discovers current windows and immediately reapplies every selected placement even when Auto is off. Enabling **Auto** also immediately applies poker and ordinary-window placement. Native discovery events use a 200 ms trailing debounce.

Mixed columns use the monitor's full working height when they fit. LDPlayer keeps at least Pokerrr 2's natural 9:16 portrait ratio and retains any wider detected ratio, so repeated Auto reflows cannot make it progressively narrower. When several columns no longer fit, the layout reduces their shared height while preserving ratios and preventing overlap.

**Fill space** gives poker priority. Poker geometry is calculated first, then the application is resized into the exact remaining full-height strip from the actual or preserved poker-column boundary to the selected monitor's right edge—even when that strip is narrower than the application's reported minimum size. It never remains overlapping poker or uses a band below it. Multiple Fill-space applications share that rectangle.

Default shortcuts:

- `Ctrl+Shift+A` — arrange now.
- `Ctrl+Shift+P` — show or hide the panel.
- `Ctrl+Shift+G` — locate detected ClubGG lobbies sequentially.

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
