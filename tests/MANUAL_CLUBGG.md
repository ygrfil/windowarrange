# Manual ClubGG acceptance

Run these checks only by explicitly launching the release executable while the user is present. Automated tests must never execute this checklist or move real ClubGG windows.

1. Open the ClubGG lobby and one table. Confirm only the table is managed and the table starts at the selected display's top-left.
2. Repeat with 2 through 8 tables. Confirm every active table has equal outer dimensions, retains its natural shape, fits the working area, and avoids the taskbar.
3. With five tables, close the middle table. Confirm the remaining four keep relative order and immediately refit to the best four-table layout.
4. Disable a table in the panel and through its numbered shortcut. Confirm it shrinks and parks at the selected display's bottom-right while active tables refit.
5. Re-enable the parked table. Confirm it returns to its prior ordered slot and all active tables refit equally.
6. Use the fixed arrow controls to reorder tables. Confirm the windows move without stealing focus and the other row controls remain in place.
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
17. If Asian Hand Converter is installed, leave it running and launch `Table-Arranger-Control.exe`. Confirm the neutral control panel remains open or available in the tray and can arrange repeatedly without a converter DLL crash.
18. Confirm `%APPDATA%\ClubGGTools\ClubGG Table Arranger\config\logs\table-arranger.log` records startup, discovery-count changes, and arrangement results without full ClubGG table titles.
19. Open several ordinary applications. Confirm every normal top-level window appears, the arranger itself and Windows desktop/taskbar surfaces do not appear, and every new ordinary window defaults to **Ignore** without moving.
20. Choose **Top-right** for an ordinary window. Confirm its size is preserved, it moves to the selected display's top-right without activation, and the choice survives restart.
21. Choose **Fill space** with 1–8 active poker tables. Confirm the application fills only the full-height strip to the right of the rightmost active table, never a band below the tables, without changing poker-table size or placement. Confirm multiple Fill-space windows share that strip.
22. Save Top-right or Fill-space for an ordinary window, exit through the tray, change the window title if possible, and relaunch. Confirm the saved behavior is restored. Confirm an exact rule still wins when two identifiable windows from one application have different choices.
23. With Auto off, manually move a selected ordinary window and press **Refresh**. Confirm discovery and selected poker/ordinary placement are immediately reapplied. Turn Auto on and confirm the complete workspace is immediately reapplied again.
24. Drag a managed poker tile onto another tile in the compact poker board. Confirm their numbered screen slots swap immediately without focus theft. Restart and confirm the dragged order is restored.
25. Confirm the compact horizontal panel retains Auto, Arrange, Refresh, Settings, display selection, status, Locate, tray hiding, poker Active/Park/Ignore, and ordinary Ignore/Fill-space/Top-right controls without requiring the old tall all-window list.
26. Upgrade from 0.4.1 after it was resized. Confirm 0.4.2 opens at its smaller default size, cannot maximize, paints the entire client background, shows all four rail actions without overlap or missing-glyph boxes, and keeps both panes bounded at minimum, default, and maximum sizes.
27. Open 1–8 tables and several ordinary windows. Confirm all current entries and controls are visible simultaneously without scrolling, the dense two-column grids stay inside their panes, and no content stretches or clips the other pane.
28. Mix active, parked, and ignored ClubGG windows. Confirm the poker pane has separate **Active tables** and **Inactive tables** groups, active cards remain above inactive cards, the redundant overall Poker heading is absent, and the ordinary-window pane aligns to the top of the workspace.
29. Confirm the header remains a single compact row and the Auto/Arrange/Refresh/Settings rail, both table groups, and ordinary-window cards appear immediately below it. Resize between minimum and maximum and confirm the header never displaces the workspace outside the client area.
30. Confirm no Active tables, Inactive tables, Other, or saved-choices descriptions appear. The first poker and ordinary-window cards must align with the top of Auto, while a thin divider still separates active from inactive poker cards.
