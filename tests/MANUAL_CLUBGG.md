# Manual poker-client acceptance

Run these checks only by explicitly launching the release executable while the user is present. Automated tests must never execute this checklist or move real ClubGG or LDPlayer windows.

1. Open the ClubGG lobby and one table. Confirm only the table defaults to Active and it occupies the top half of the first ClubGG column.
2. Repeat with 2 through 8 ClubGG tables. Confirm slots run down columns `(1/2)`, `(3/4)`, `(5/6)`, every ClubGG table remains equal and 4:3, and an odd final table leaves the lower half empty.
3. Open Pokerrr 2 in LDPlayer 9. Confirm the eligible `dnplayer.exe` main window appears as an Active poker table while LDPlayer headless/service processes do not appear.
4. With two ClubGG tables and one LDPlayer table, confirm the ClubGG pair shares the first full-height column and LDPlayer occupies its own full-height column to the right at its detected aspect ratio.
5. Add more ClubGG and LDPlayer windows until full-height columns no longer fit. Confirm the mixed layout becomes shorter without changing either client aspect ratio or overlapping windows.
6. Confirm the poker pane is a scaled spatial mirror of the selected monitor: tile position, relative size, empty lower halves, LDPlayer full-height columns, and parked outlines match the desktop. Confirm untitled or simply titled `ClubGG` shell windows do not appear as table tiles.
7. Confirm every poker tile uses small painter-drawn icons for Locate, Active, Park, and Ignore with correct tooltips and no missing-glyph boxes. Confirm ordinary application cards use matching Locate, Ignore, Fill-space, and Top-right icons and Settings explains all six.
8. Select a ClubGG table and click another ClubGG table. Confirm only those two real windows swap and the change survives restart.
9. With table 1 above placeholder 2, select table 1 and click placeholder 2. Confirm it moves there, its previous position becomes a dashed tile explicitly named Placeholders with a muted number badge, and the hole survives Arrange, Auto reflow, and restart while preservation is effective.
10. Select LDPlayer and click either member of a ClubGG column. Confirm the entire LDPlayer column swaps with the ClubGG pair or single-plus-empty column while the pair keeps its top/bottom order.
11. Click the selected table again and confirm selection is cancelled without moving anything.
12. Park a ClubGG and an LDPlayer table. Confirm each real window uses its own minimum supported size in the bottom-right parking row, while an orange owned ghost remains in its active slot and the named actual parked miniature appears at the mirrored bottom-right. Left-click that miniature and confirm Locate runs; right-click it and confirm the table unparks.
13. Swap an active table with a parked-table ghost. Confirm only slot reservations change; the parked window stays parked until explicitly activated.
14. Close a table. Confirm its identity disappears immediately while its position becomes an anonymous empty placeholder when Preserve table slots is on.
15. Set a poker table to Ignore. Confirm its reservation is released and it remains accessible with Locate/Active/Park/Ignore controls in the compact unmanaged row.
16. Confirm Preserve table slots is on by default and never adds anonymous space beyond two total columns. With one ClubGG column, open LDPlayer and confirm it consumes the second reserved column rather than creating a third. At two or more active columns confirm preservation is reported inactive, but its checkbox remains clickable; close or Park below two and confirm the saved On preference becomes effective again.
17. With fewer than two occupied columns, turn Preserve table slots off manually while Auto is off. Confirm holes disappear, active poker windows compact immediately, and Fill-space applications immediately use the actual poker boundary. Open/close or Park/unpark more tables and confirm the manual Off choice remains off. Turn it on again and confirm the minimum two-column footprint returns without creating or moving fake windows.
18. Open a new ClubGG table with an empty half-slot available. Confirm it fills the earliest compatible placeholder. Open a new LDPlayer main window and confirm it receives a separate column after the default ClubGG section.
19. Manually move one arranged poker table. Confirm it remains there until another structural reflow, slot change, or explicit Arrange action.
20. Change target monitors and disconnect the selected monitor. Confirm fallback to the primary monitor and that the mirrored board updates to the chosen working area.
21. Exercise every configurable global shortcut and verify conflicts are shown without disabling panel controls. Confirm each actual window retains one numbered badge/hotkey slot even though LDPlayer spans a full-height column.
22. Use Locate on ClubGG and LDPlayer. Confirm the chosen window restores, raises, requests foreground focus, and only flashes when Windows denies activation.
23. Close and minimize the panel to the tray, restore it by tray click and shortcut, then use tray Exit. Confirm parked tables remain parked.
24. Launch the release executable and confirm Windows shows UAC before the panel starts. Confirm debug/test verification remains runnable without UAC.
25. If Asian Hand Converter is installed, leave it running and launch `Table-Arranger-Control.exe`. Confirm the neutral control panel remains open or available in the tray and arranges repeatedly without a converter DLL crash.
26. Confirm `%APPDATA%\ClubGGTools\ClubGG Table Arranger\config\logs\table-arranger.log` records startup, discovery-count changes, and arrangement results without full ClubGG or LDPlayer titles.
27. Open several ordinary applications. Confirm normal top-level windows appear separately, default to Ignore, and retain Ignore/Free/Top choices across restart and title changes.
28. Choose Top-right for an ordinary window. Confirm its size is preserved and it moves to the selected display's top-right without activation.
29. Choose Fill space. Confirm it uses only the full-height strip to the right of the rightmost actual or preserved poker column, never a band below poker tables, and multiple Fill-space windows share the strip.
30. With Auto off, press Arrange and confirm discovery plus all selected poker and ordinary placement reapplies immediately. Enable Auto and confirm the complete workspace applies immediately again.
31. Confirm the top toolbar contains only Auto, Arrange, Settings, and tray hiding. Confirm display selection, Preserve table slots, icon legend, ordinary-window default, status/counts, and hotkeys remain in the independent Settings window.
32. Open 1–8 poker windows and several ordinary windows. Confirm every mirrored tile, placeholder, unmanaged poker control, and ordinary card remains visible without scroll areas or pane overlap, and the panel height shrinks and grows without clipping.
33. Expand and collapse Settings hotkeys. Confirm the independent non-resizable Settings window refits without inflating or continuously repainting the main panel.
34. Confirm the executable, taskbar window, and tray all show the same royal-blue icon with a dark-blue spade silhouette and centered white plus.
35. Confirm idle CPU, RAM, and GPU use remain proportionate to a compact utility and that discovery still reacts after the 200 ms native-event debounce.
