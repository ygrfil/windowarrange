# Manual ClubGG acceptance

Run these checks only by explicitly launching the release executable while the user is present. Automated tests must never execute this checklist or move real ClubGG windows.

1. Open the ClubGG lobby and one table. Confirm only the table is managed and the table starts at the selected display's top-left.
2. Repeat with 2 through 8 tables. Confirm every active table has equal outer dimensions, retains its natural shape, fits the working area, and avoids the taskbar.
3. With five tables, close the middle table. Confirm the remaining four keep relative order and immediately refit to the best four-table layout.
4. Disable several tables in the panel and through numbered shortcuts. Confirm they shrink to their smallest supported sizes and line up without overlap from the selected display's bottom-right toward the left while active tables refit.
5. Re-enable the parked table. Confirm it returns to its prior ordered slot and all active tables refit equally.
6. Use the table-number click controls to reorder tables. Confirm the windows move without stealing focus and the other row controls remain in place.
7. Manually move one arranged table. Confirm it remains there until another table opens/closes/toggles, order changes, or **Arrange now** is used.
8. Open a new table. Confirm it is enabled, appended, and included after the 500 ms debounce.
9. Confirm every discovered ClubGG window appears in one list. Use **◎** to identify the lobby and tables, set the lobby to **Ignore**, a table to **Park**, and the remaining tables to **Arrange**; restart and confirm classifications are remembered.
10. Change target monitors and disconnect the selected monitor. Confirm fallback to the primary monitor.
11. Exercise every configurable global shortcut and verify conflicts are shown without disabling panel controls.
12. Close and minimize the panel to the tray, restore it by tray click and shortcut, then use tray **Exit**. Confirm parked tables remain parked.
13. Launch the release executable and confirm Windows always shows UAC before the panel starts. Confirm debug/test verification remains runnable without UAC.
14. Arrange tables and confirm the panel row order and all row controls remain fixed even though the table screen coordinates change.
15. Confirm the panel groups all arranged windows first, parked windows next, and ignored windows last while preserving relative order inside each group.
16. With two tables, confirm they use the four-table size in the top-left and top-right slots. With three, confirm the third occupies bottom-left at the same size. Confirm four tables fill the same 2×2 slots.
17. With five through eight tables, confirm slots 1–4 remain `(1,2)/(3,4)`, slot 5 appears above 6 in the next column, and slot 7 appears above 8 in the following column.
18. If Asian Hand Converter is installed, leave it running and launch `Table-Arranger-Control.exe`. Confirm the neutral control panel remains open or available in the tray and can arrange repeatedly without a converter DLL crash.
19. Confirm `%APPDATA%\ClubGGTools\ClubGG Table Arranger\config\logs\table-arranger.log` records startup, discovery-count changes, and arrangement results without full ClubGG table titles.
20. Open several ordinary applications. Confirm every normal top-level window appears, the arranger itself and Windows desktop/taskbar surfaces do not appear, and every new ordinary window initially defaults to **Ignore** without moving. Change the global default in Settings to **Free** and **Top**, opening a new ordinary window after each change; confirm the new default applies immediately and survives restart, while an explicitly saved card choice still wins.
21. Choose **Top-right** for an ordinary window. Confirm its size is preserved, it moves to the selected display's top-right without activation, and the choice survives restart.
22. Choose **Fill space** with 1–8 active poker tables. Confirm the application fills only the full-height strip to the right of the rightmost active table, never a band below the tables, without changing poker-table size or placement. Confirm multiple Fill-space windows share that strip.
23. Save Top-right or Fill-space for an ordinary window, exit through the tray, change the window title if possible, and relaunch. Confirm the saved behavior is restored. Confirm an exact rule still wins when two identifiable windows from one application have different choices.
24. With Auto off, manually move a selected ordinary window and press **Arrange**. Confirm discovery and selected poker/ordinary placement are immediately reapplied. Turn Auto on and confirm the complete workspace is immediately reapplied again.
25. Click one managed poker table number, then click a different table number in the compact poker board. Confirm the first table shows a selected outline and their numbered screen slots swap immediately without focus theft. Select a number and click it again to confirm selection is cancelled. Restart and confirm the swapped order is restored.
26. Confirm the compact panel's top toolbar contains only Auto, Arrange, Settings, and tray hiding. Confirm display selection, 2 Slots, ordinary-window default, status/counts, and hotkey details are in Settings, while Locate and per-window modes remain on the cards.
27. Upgrade from an older resized build. Confirm 0.5.0 discards stale panel geometry, cannot maximize, paints the entire client background, shows all toolbar actions without overlap or missing-glyph boxes, and keeps both panes bounded.
28. Open 1–8 tables and several ordinary windows. Confirm all current entries and controls are visible simultaneously without scrolling, the dense two-column grids stay inside their panes, and no content stretches or clips the other pane.
29. Mix active, parked, and ignored ClubGG windows. Confirm the poker pane has separate **Active tables** and **Inactive tables** groups, active cards remain above inactive cards, the redundant overall Poker heading is absent, and the ordinary-window pane aligns to the top of the workspace.
30. Confirm the toolbar remains a single compact row and both table groups and ordinary-window cards appear immediately below it. With two active tables, three inactive tables, and three ordinary windows, confirm the bottom inactive card is fully visible. Confirm panel height automatically shrinks when cards close and grows only as needed, without leaving a large blank band under the last row.
31. Confirm no Workspace, counts, status, Active tables, Inactive tables, Other, or saved-choices descriptions appear on the main board. A thin divider must still separate active from inactive poker cards; the removed information must remain available in Settings.
32. Confirm the executable, taskbar window, and tray all show the same royal-blue icon with a dark-blue spade silhouette and a centered white plus.
33. With a Fill-space application selected, test zero and one active poker table. Confirm 2 Slots is on by default and reserves exactly the normal two-table width. Turn it off while Auto is off and confirm the application immediately reclaims the unreserved right-side space; restart and confirm the switch choice is remembered. Confirm no placeholder poker window is created or moved.
