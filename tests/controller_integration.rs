use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use clubgg_table_arranger::{
    config::{AppConfig, ApplicationDefault, ConfigStore},
    controller::{ControllerCommand, spawn_controller},
    layout::{calculate_layout, right_side_free_rect},
    model::{
        BackendError, MonitorInfo, PokerClientKind, PokerColumnAssignment, PokerSlotId, Rect, Size,
        TableStatus, UiSnapshot, WindowBackend, WindowCandidate, WindowId, WindowMode,
        WindowSignature,
    },
};

#[derive(Clone)]
struct MockBackend {
    candidates: Arc<Mutex<Vec<WindowCandidate>>>,
    moves: Arc<Mutex<Vec<(WindowId, Rect)>>>,
    located: Arc<Mutex<Vec<WindowId>>>,
    foreground: Arc<Mutex<Option<WindowId>>>,
    deny_moves: Arc<AtomicBool>,
}

impl MockBackend {
    fn with_tables(count: usize) -> Self {
        let candidates = (0..count).map(|index| candidate(index + 1)).collect();
        Self {
            candidates: Arc::new(Mutex::new(candidates)),
            moves: Arc::new(Mutex::new(Vec::new())),
            located: Arc::new(Mutex::new(Vec::new())),
            foreground: Arc::new(Mutex::new(None)),
            deny_moves: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl WindowBackend for MockBackend {
    fn enumerate_candidates(&self) -> Result<Vec<WindowCandidate>, BackendError> {
        Ok(self.candidates.lock().unwrap().clone())
    }

    fn monitors(&self) -> Result<Vec<MonitorInfo>, BackendError> {
        Ok(vec![MonitorInfo {
            id: "test-monitor".to_owned(),
            label: "Test monitor".to_owned(),
            work_area: Rect::new(0, 0, 2752, 1104),
            primary: true,
        }])
    }

    fn move_resize(&self, id: WindowId, rect: Rect) -> Result<Rect, BackendError> {
        if self.deny_moves.load(Ordering::Relaxed) {
            return Err(BackendError::AccessDenied);
        }
        self.moves.lock().unwrap().push((id, rect));
        Ok(rect)
    }

    fn minimum_size(&self, _id: WindowId, _aspect_ratio: f64) -> Result<Size, BackendError> {
        Ok(Size::new(240, 180))
    }

    fn locate(&self, id: WindowId) -> Result<(), BackendError> {
        self.located.lock().unwrap().push(id);
        Ok(())
    }

    fn foreground_window(&self) -> Option<WindowId> {
        *self.foreground.lock().unwrap()
    }
}

#[test]
fn locate_command_reaches_the_selected_window() {
    let backend = MockBackend::with_tables(1);
    let store = ConfigStore::at(temp_config_path("locate-window"));
    let handle = spawn_controller(Arc::new(backend.clone()), AppConfig::default(), store);
    let _ = wait_for_snapshot(&handle.snapshots, |snapshot| snapshot.tables.len() == 1);

    handle
        .commands
        .send(ControllerCommand::Locate(WindowId(1)))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    while backend.located.lock().unwrap().as_slice() != [WindowId(1)] && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }

    assert_eq!(backend.located.lock().unwrap().as_slice(), [WindowId(1)]);
    handle.commands.send(ControllerCommand::Shutdown).unwrap();
}

#[test]
fn closing_disabling_and_reordering_preserve_relative_order() {
    let backend = MockBackend::with_tables(5);
    let store = ConfigStore::at(temp_config_path("state-transitions"));
    let handle = spawn_controller(Arc::new(backend.clone()), AppConfig::default(), store);

    let initial = wait_for_snapshot(&handle.snapshots, |snapshot| snapshot.tables.len() == 5);
    assert_eq!(ids(&initial), vec![1, 2, 3, 4, 5]);

    handle
        .commands
        .send(ControllerCommand::SetEnabled {
            id: WindowId(2),
            enabled: false,
        })
        .unwrap();
    let disabled = wait_for_snapshot(&handle.snapshots, |snapshot| {
        snapshot
            .tables
            .iter()
            .any(|table| table.id == WindowId(2) && !table.enabled)
    });
    assert_eq!(ids(&disabled), vec![1, 2, 3, 4, 5]);
    assert!(
        backend
            .moves
            .lock()
            .unwrap()
            .iter()
            .any(|(id, rect)| *id == WindowId(2) && *rect == Rect::new(2512, 924, 240, 180))
    );

    handle
        .commands
        .send(ControllerCommand::MoveToSlot {
            source: WindowId(5),
            destination: PokerSlotId::club(0, 0),
        })
        .unwrap();
    let reordered = wait_for_snapshot(&handle.snapshots, |snapshot| {
        ids(snapshot) == vec![5, 2, 3, 4, 1]
    });
    assert_eq!(ids(&reordered), vec![5, 2, 3, 4, 1]);

    backend
        .candidates
        .lock()
        .unwrap()
        .retain(|item| item.id != WindowId(3));
    handle
        .commands
        .send(ControllerCommand::ForceArrange)
        .unwrap();
    let closed = wait_for_snapshot(&handle.snapshots, |snapshot| snapshot.tables.len() == 4);
    assert_eq!(ids(&closed), vec![5, 2, 4, 1]);

    handle.commands.send(ControllerCommand::Shutdown).unwrap();
}

#[test]
fn ldplayer_uses_a_full_height_column_and_cross_swap_moves_the_club_pair() {
    let backend = MockBackend::with_tables(2);
    backend
        .candidates
        .lock()
        .unwrap()
        .push(ldplayer_candidate(9));
    let store = ConfigStore::at(temp_config_path("mixed-ldplayer-column"));
    let config = AppConfig {
        auto_arrange: false,
        ..AppConfig::default()
    };
    let handle = spawn_controller(Arc::new(backend.clone()), config, store.clone());
    let initial = wait_for_snapshot(&handle.snapshots, |snapshot| snapshot.tables.len() == 3);

    assert_eq!(ids(&initial), vec![1, 2, 9]);
    assert!(!initial.preserve_table_slots);
    assert!(initial.preserve_table_slots_requested);
    assert!(initial.preserve_table_slots_auto_suppressed);
    assert_eq!(initial.poker_slots.len(), 3);
    let ld_slot = initial
        .poker_slots
        .iter()
        .find(|slot| slot.occupant == Some(WindowId(9)))
        .unwrap();
    assert_eq!(ld_slot.id, PokerSlotId::full_height(1));
    assert_eq!(ld_slot.rect.height, 1104);
    let saved = store.load().unwrap();
    assert_eq!(saved.poker_columns.len(), 2);
    assert!(matches!(
        saved.poker_columns.get(1),
        Some(PokerColumnAssignment::LdPlayer { table: Some(_) })
    ));

    handle
        .commands
        .send(ControllerCommand::MoveToSlot {
            source: WindowId(9),
            destination: PokerSlotId::club(0, 0),
        })
        .unwrap();
    let swapped = wait_for_snapshot(&handle.snapshots, |snapshot| ids(snapshot) == vec![9, 1, 2]);
    assert_eq!(
        swapped
            .poker_slots
            .iter()
            .find(|slot| slot.occupant == Some(WindowId(9)))
            .unwrap()
            .id,
        PokerSlotId::full_height(0)
    );
    assert_eq!(
        swapped
            .poker_slots
            .iter()
            .find(|slot| slot.occupant == Some(WindowId(1)))
            .unwrap()
            .id,
        PokerSlotId::club(1, 0)
    );
    assert_eq!(
        swapped
            .poker_slots
            .iter()
            .find(|slot| slot.occupant == Some(WindowId(2)))
            .unwrap()
            .id,
        PokerSlotId::club(1, 1)
    );

    handle.commands.send(ControllerCommand::Shutdown).unwrap();
}

#[test]
fn full_height_ldplayer_never_reflows_narrower_than_its_natural_ratio() {
    let backend = MockBackend::with_tables(0);
    let mut ldplayer = ldplayer_candidate(9);
    ldplayer.rect = Rect::new(0, 0, 500, 1104);
    ldplayer.preferred_aspect_ratio = 0.50;
    backend.candidates.lock().unwrap().push(ldplayer);
    let store = ConfigStore::at(temp_config_path("natural-ldplayer-ratio"));
    let config = AppConfig {
        auto_arrange: false,
        ..AppConfig::default()
    };
    let handle = spawn_controller(Arc::new(backend.clone()), config, store);
    let snapshot = wait_for_snapshot(&handle.snapshots, |snapshot| snapshot.tables.len() == 1);
    let slot = snapshot
        .poker_slots
        .iter()
        .find(|slot| slot.occupant == Some(WindowId(9)))
        .unwrap();

    assert_eq!(slot.rect.height, 1104);
    assert_eq!(slot.rect.width, 621);

    handle
        .commands
        .send(ControllerCommand::ForceArrange)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    while !backend
        .moves
        .lock()
        .unwrap()
        .iter()
        .any(|(id, rect)| *id == WindowId(9) && rect.width == 621 && rect.height == 1104)
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        backend
            .moves
            .lock()
            .unwrap()
            .iter()
            .any(|(id, rect)| *id == WindowId(9) && rect.width == 621 && rect.height == 1104)
    );

    handle.commands.send(ControllerCommand::Shutdown).unwrap();
}

#[test]
fn existing_ldplayer_assignment_reclaims_an_older_reserved_middle_column() {
    let backend = MockBackend::with_tables(1);
    let ldplayer = ldplayer_candidate(9);
    backend.candidates.lock().unwrap().push(ldplayer.clone());
    let club_signature = backend.candidates.lock().unwrap()[0].signature.clone();
    let mut config = AppConfig {
        auto_arrange: false,
        ..AppConfig::default()
    };
    config.poker_columns = vec![
        PokerColumnAssignment::ClubGg {
            top: Some(club_signature),
            bottom: None,
        },
        PokerColumnAssignment::empty_club(),
        PokerColumnAssignment::LdPlayer {
            table: Some(ldplayer.signature),
        },
    ];
    let store = ConfigStore::at(temp_config_path("ldplayer-reclaims-middle-reservation"));
    let handle = spawn_controller(Arc::new(backend), config, store.clone());
    let snapshot = wait_for_snapshot(&handle.snapshots, |snapshot| snapshot.tables.len() == 2);

    assert_eq!(
        snapshot
            .poker_slots
            .iter()
            .find(|slot| slot.occupant == Some(WindowId(9)))
            .unwrap()
            .id,
        PokerSlotId::full_height(1)
    );
    let saved = store.load().unwrap();
    assert_eq!(saved.poker_columns.len(), 2);
    assert!(saved.poker_columns.iter().all(|column| {
        !matches!(
            column,
            PokerColumnAssignment::Empty
                | PokerColumnAssignment::ClubGg {
                    top: None,
                    bottom: None
                }
        )
    }));

    handle.commands.send(ControllerCommand::Shutdown).unwrap();
}

#[test]
fn preserved_anonymous_columns_are_capped_at_two_total_columns() {
    let backend = MockBackend::with_tables(1);
    let signature = backend.candidates.lock().unwrap()[0].signature.clone();
    let mut config = AppConfig {
        auto_arrange: false,
        ..AppConfig::default()
    };
    config.poker_columns = vec![
        PokerColumnAssignment::empty_club(),
        PokerColumnAssignment::ClubGg {
            top: Some(signature),
            bottom: None,
        },
        PokerColumnAssignment::empty_club(),
        PokerColumnAssignment::Empty,
    ];
    let store = ConfigStore::at(temp_config_path("two-column-preservation-cap"));
    let handle = spawn_controller(Arc::new(backend), config, store.clone());
    let snapshot = wait_for_snapshot(&handle.snapshots, |snapshot| snapshot.tables.len() == 1);

    assert!(snapshot.preserve_table_slots);
    assert_eq!(store.load().unwrap().poker_columns.len(), 2);
    assert_eq!(
        snapshot.poker_slots.iter().map(|slot| slot.id.column).max(),
        Some(1)
    );

    handle.commands.send(ControllerCommand::Shutdown).unwrap();
}

#[test]
fn initial_discovery_keeps_clubgg_before_ldplayer_even_when_ldplayer_is_seen_first() {
    let backend = MockBackend::with_tables(0);
    backend
        .candidates
        .lock()
        .unwrap()
        .extend([ldplayer_candidate(9), candidate(1)]);
    let store = ConfigStore::at(temp_config_path("club-before-ld-discovery-order"));
    let handle = spawn_controller(Arc::new(backend), AppConfig::default(), store.clone());
    let snapshot = wait_for_snapshot(&handle.snapshots, |snapshot| snapshot.tables.len() == 2);

    assert_eq!(ids(&snapshot), vec![1, 9]);
    assert_eq!(store.load().unwrap().poker_columns.len(), 2);
    assert_eq!(
        snapshot
            .poker_slots
            .iter()
            .find(|slot| slot.occupant == Some(WindowId(1)))
            .unwrap()
            .id,
        PokerSlotId::club(0, 0)
    );
    assert_eq!(
        snapshot
            .poker_slots
            .iter()
            .find(|slot| slot.occupant == Some(WindowId(9)))
            .unwrap()
            .id,
        PokerSlotId::full_height(1)
    );

    handle.commands.send(ControllerCommand::Shutdown).unwrap();
}

#[test]
fn moving_table_one_into_placeholder_two_moves_the_window_and_leaves_a_hole() {
    let backend = MockBackend::with_tables(1);
    let store = ConfigStore::at(temp_config_path("move-into-placeholder-two"));
    let config = AppConfig {
        auto_arrange: false,
        ..AppConfig::default()
    };
    let handle = spawn_controller(Arc::new(backend), config, store);
    let initial = wait_for_snapshot(&handle.snapshots, |snapshot| snapshot.tables.len() == 1);
    assert!(
        initial.poker_slots.iter().any(|slot| {
            slot.id == PokerSlotId::club(0, 0) && slot.occupant == Some(WindowId(1))
        })
    );
    assert!(
        initial
            .poker_slots
            .iter()
            .any(|slot| { slot.id == PokerSlotId::club(0, 1) && slot.occupant.is_none() })
    );

    handle
        .commands
        .send(ControllerCommand::MoveToSlot {
            source: WindowId(1),
            destination: PokerSlotId::club(0, 1),
        })
        .unwrap();
    let moved = wait_for_snapshot(&handle.snapshots, |snapshot| {
        snapshot
            .poker_slots
            .iter()
            .any(|slot| slot.id == PokerSlotId::club(0, 1) && slot.occupant == Some(WindowId(1)))
    });
    assert!(
        moved
            .poker_slots
            .iter()
            .any(|slot| { slot.id == PokerSlotId::club(0, 0) && slot.occupant.is_none() })
    );

    handle.commands.send(ControllerCommand::Shutdown).unwrap();
}

#[test]
fn preserve_slots_auto_suppresses_for_two_busy_columns_and_restores_after_park() {
    let backend = MockBackend::with_tables(1);
    backend
        .candidates
        .lock()
        .unwrap()
        .push(ldplayer_candidate(9));
    let store = ConfigStore::at(temp_config_path("automatic-slot-preservation"));
    let handle = spawn_controller(Arc::new(backend), AppConfig::default(), store.clone());
    let suppressed = wait_for_snapshot(&handle.snapshots, |snapshot| {
        snapshot.tables.len() == 2 && snapshot.preserve_table_slots_auto_suppressed
    });
    assert!(!suppressed.preserve_table_slots);
    assert!(suppressed.preserve_table_slots_requested);
    assert!(store.load().unwrap().preserve_table_slots);

    handle
        .commands
        .send(ControllerCommand::SetPreserveTableSlots(false))
        .unwrap();
    let manually_disabled = wait_for_snapshot(&handle.snapshots, |snapshot| {
        !snapshot.preserve_table_slots && !snapshot.preserve_table_slots_requested
    });
    assert!(!manually_disabled.preserve_table_slots_auto_suppressed);
    assert!(!store.load().unwrap().preserve_table_slots);

    handle
        .commands
        .send(ControllerCommand::SetEnabled {
            id: WindowId(9),
            enabled: false,
        })
        .unwrap();
    let still_disabled = wait_for_snapshot(&handle.snapshots, |snapshot| {
        !snapshot.preserve_table_slots
            && !snapshot.preserve_table_slots_requested
            && snapshot
                .tables
                .iter()
                .any(|table| table.id == WindowId(9) && !table.enabled)
    });
    assert!(!still_disabled.preserve_table_slots_auto_suppressed);

    handle
        .commands
        .send(ControllerCommand::SetPreserveTableSlots(true))
        .unwrap();
    let restored = wait_for_snapshot(&handle.snapshots, |snapshot| {
        snapshot.preserve_table_slots && snapshot.preserve_table_slots_requested
    });
    assert!(!restored.preserve_table_slots_auto_suppressed);

    handle.commands.send(ControllerCommand::Shutdown).unwrap();
}

#[test]
fn parked_table_keeps_a_ghost_and_closed_table_releases_its_owner() {
    let backend = MockBackend::with_tables(2);
    let store = ConfigStore::at(temp_config_path("parked-slot-ghost"));
    let handle = spawn_controller(Arc::new(backend.clone()), AppConfig::default(), store);
    let _ = wait_for_snapshot(&handle.snapshots, |snapshot| snapshot.tables.len() == 2);

    handle
        .commands
        .send(ControllerCommand::SetEnabled {
            id: WindowId(1),
            enabled: false,
        })
        .unwrap();
    let parked = wait_for_snapshot(&handle.snapshots, |snapshot| {
        snapshot
            .poker_slots
            .iter()
            .any(|slot| slot.occupant == Some(WindowId(1)) && slot.parked)
    });
    assert_eq!(
        parked
            .candidates
            .iter()
            .find(|candidate| candidate.id == WindowId(1))
            .unwrap()
            .current_rect,
        Rect::new(2512, 924, 240, 180)
    );

    backend
        .candidates
        .lock()
        .unwrap()
        .retain(|candidate| candidate.id != WindowId(1));
    handle
        .commands
        .send(ControllerCommand::ForceArrange)
        .unwrap();
    let closed = wait_for_snapshot(&handle.snapshots, |snapshot| snapshot.tables.len() == 1);
    assert!(
        closed
            .poker_slots
            .iter()
            .all(|slot| slot.occupant != Some(WindowId(1)))
    );
    assert!(
        closed
            .poker_slots
            .iter()
            .any(|slot| slot.id == PokerSlotId::club(0, 0) && slot.occupant.is_none())
    );

    handle.commands.send(ControllerCommand::Shutdown).unwrap();
}

#[test]
fn disabling_slot_preservation_compacts_manual_holes_immediately() {
    let backend = MockBackend::with_tables(2);
    let store = ConfigStore::at(temp_config_path("compact-preserved-hole"));
    let config = AppConfig {
        auto_arrange: false,
        ..AppConfig::default()
    };
    let handle = spawn_controller(Arc::new(backend), config, store);
    let _ = wait_for_snapshot(&handle.snapshots, |snapshot| snapshot.tables.len() == 2);

    handle
        .commands
        .send(ControllerCommand::MoveToSlot {
            source: WindowId(1),
            destination: PokerSlotId::club(1, 0),
        })
        .unwrap();
    let with_hole = wait_for_snapshot(&handle.snapshots, |snapshot| {
        snapshot
            .poker_slots
            .iter()
            .any(|slot| slot.id == PokerSlotId::club(0, 0) && slot.occupant.is_none())
            && snapshot.poker_slots.iter().any(|slot| {
                slot.id == PokerSlotId::club(1, 0) && slot.occupant == Some(WindowId(1))
            })
    });
    assert!(!with_hole.preserve_table_slots);
    assert!(with_hole.preserve_table_slots_requested);
    assert!(with_hole.preserve_table_slots_auto_suppressed);

    handle
        .commands
        .send(ControllerCommand::SetPreserveTableSlots(false))
        .unwrap();
    let compacted = wait_for_snapshot(&handle.snapshots, |snapshot| {
        !snapshot.preserve_table_slots && snapshot.poker_slots.len() == 2
    });
    assert!(
        compacted
            .poker_slots
            .iter()
            .all(|slot| slot.occupant.is_some())
    );

    handle.commands.send(ControllerCommand::Shutdown).unwrap();
}

#[test]
fn parked_tables_line_up_from_bottom_right_without_overlap() {
    let backend = MockBackend::with_tables(3);
    let store = ConfigStore::at(temp_config_path("parked-shoulder-row"));
    let config = AppConfig {
        auto_arrange: false,
        ..AppConfig::default()
    };
    let handle = spawn_controller(Arc::new(backend.clone()), config, store);
    let _ = wait_for_snapshot(&handle.snapshots, |snapshot| snapshot.tables.len() == 3);

    for id in 1..=3 {
        handle
            .commands
            .send(ControllerCommand::SetEnabled {
                id: WindowId(id),
                enabled: false,
            })
            .unwrap();
        let _ = wait_for_snapshot(&handle.snapshots, |snapshot| {
            snapshot
                .tables
                .iter()
                .find(|table| table.id == WindowId(id))
                .is_some_and(|table| !table.enabled && table.status == TableStatus::Parked)
        });
    }

    let moves = backend.moves.lock().unwrap();
    let last_rect = |id| {
        moves
            .iter()
            .rev()
            .find(|(candidate, _)| *candidate == WindowId(id))
            .map(|(_, rect)| *rect)
    };
    assert_eq!(last_rect(1), Some(Rect::new(2512, 924, 240, 180)));
    assert_eq!(last_rect(2), Some(Rect::new(2272, 924, 240, 180)));
    assert_eq!(last_rect(3), Some(Rect::new(2032, 924, 240, 180)));

    handle.commands.send(ControllerCommand::Shutdown).unwrap();
}

#[test]
fn force_arrange_uses_equal_rectangles() {
    let backend = MockBackend::with_tables(5);
    let store = ConfigStore::at(temp_config_path("equal-layout"));
    let handle = spawn_controller(Arc::new(backend.clone()), AppConfig::default(), store);
    let _ = wait_for_snapshot(&handle.snapshots, |snapshot| snapshot.tables.len() == 5);
    backend.moves.lock().unwrap().clear();

    handle
        .commands
        .send(ControllerCommand::ForceArrange)
        .unwrap();
    let _ = wait_for_snapshot(&handle.snapshots, |snapshot| {
        snapshot.status_message.starts_with("Arranged 5")
    });
    let moves = backend.moves.lock().unwrap().clone();
    let final_moves = &moves[moves.len() - 5..];
    let first = final_moves[0].1;
    assert!(
        final_moves
            .iter()
            .all(|(_, rect)| rect.width == first.width && rect.height == first.height)
    );
    assert_eq!((first.left, first.top), (0, 0));
    handle.commands.send(ControllerCommand::Shutdown).unwrap();
}

#[test]
fn force_arrange_reports_access_denied_instead_of_claiming_success() {
    let backend = MockBackend::with_tables(2);
    backend.deny_moves.store(true, Ordering::Relaxed);
    let store = ConfigStore::at(temp_config_path("access-denied"));
    let config = AppConfig {
        auto_arrange: false,
        ..AppConfig::default()
    };
    let handle = spawn_controller(Arc::new(backend), config, store);
    let _ = wait_for_snapshot(&handle.snapshots, |snapshot| snapshot.tables.len() == 2);

    handle
        .commands
        .send(ControllerCommand::ForceArrange)
        .unwrap();
    let denied = wait_for_snapshot(&handle.snapshots, |snapshot| {
        snapshot.status_message.contains("2 access denied")
    });
    assert!(
        denied
            .tables
            .iter()
            .all(|table| table.status == TableStatus::AccessDenied)
    );
    handle.commands.send(ControllerCommand::Shutdown).unwrap();
}

#[test]
fn discovered_windows_keep_session_order_after_screen_positions_change() {
    let backend = MockBackend::with_tables(3);
    let store = ConfigStore::at(temp_config_path("stable-discovery-order"));
    let handle = spawn_controller(Arc::new(backend.clone()), AppConfig::default(), store);
    let initial = wait_for_snapshot(&handle.snapshots, |snapshot| snapshot.candidates.len() == 3);
    assert_eq!(candidate_ids(&initial), vec![1, 2, 3]);
    while handle.snapshots.try_recv().is_ok() {}

    {
        let mut candidates = backend.candidates.lock().unwrap();
        candidates.reverse();
        for (index, candidate) in candidates.iter_mut().enumerate() {
            candidate.rect.left = i32::try_from(index).unwrap() * 100;
        }
    }
    handle
        .commands
        .send(ControllerCommand::ForceArrange)
        .unwrap();
    let refreshed = wait_for_snapshot(&handle.snapshots, |snapshot| snapshot.candidates.len() == 3);
    assert_eq!(candidate_ids(&refreshed), vec![1, 2, 3]);
    handle.commands.send(ControllerCommand::Shutdown).unwrap();
}

#[test]
fn every_candidate_exposes_arrange_park_and_ignore_state() {
    let backend = MockBackend::with_tables(3);
    backend.candidates.lock().unwrap()[2].likely_table = false;
    let store = ConfigStore::at(temp_config_path("unified-window-state"));
    let handle = spawn_controller(
        Arc::new(backend.clone()),
        AppConfig::default(),
        store.clone(),
    );
    let initial = wait_for_snapshot(&handle.snapshots, |snapshot| snapshot.candidates.len() == 3);
    assert_eq!(initial.tables.len(), 2);
    assert_eq!(candidate_mode(&initial, 1), WindowMode::Arranged);
    assert_eq!(candidate_mode(&initial, 2), WindowMode::Arranged);
    assert_eq!(candidate_mode(&initial, 3), WindowMode::Ignored);

    handle
        .commands
        .send(ControllerCommand::SetWindowMode {
            id: WindowId(2),
            mode: WindowMode::Parked,
        })
        .unwrap();
    let parked = wait_for_snapshot(&handle.snapshots, |snapshot| {
        candidate_mode(snapshot, 2) == WindowMode::Parked
    });
    assert_eq!(parked.tables.len(), 2);
    assert!(!parked.tables[1].enabled);
    assert_eq!(
        store
            .load()
            .unwrap()
            .disposition_for(&candidate(2).signature),
        Some(clubgg_table_arranger::model::CandidateDisposition::Parked)
    );

    handle
        .commands
        .send(ControllerCommand::SetWindowMode {
            id: WindowId(3),
            mode: WindowMode::Arranged,
        })
        .unwrap();
    let included = wait_for_snapshot(&handle.snapshots, |snapshot| {
        candidate_mode(snapshot, 3) == WindowMode::Arranged
    });
    assert_eq!(included.tables.len(), 3);

    handle
        .commands
        .send(ControllerCommand::SetWindowMode {
            id: WindowId(1),
            mode: WindowMode::Ignored,
        })
        .unwrap();
    let ignored = wait_for_snapshot(&handle.snapshots, |snapshot| {
        candidate_mode(snapshot, 1) == WindowMode::Ignored
    });
    assert_eq!(ignored.tables.len(), 2);
    assert_eq!(ignored.candidates.len(), 3);
    assert_eq!(
        store
            .load()
            .unwrap()
            .disposition_for(&candidate(1).signature),
        Some(clubgg_table_arranger::model::CandidateDisposition::Ignored)
    );
    handle.commands.send(ControllerCommand::Shutdown).unwrap();
}

#[test]
fn saved_parked_clubgg_lobby_remains_visible_and_managed() {
    let backend = MockBackend::with_tables(0);
    let mut lobby = candidate(8);
    lobby.label = "ClubGG lobby".to_owned();
    lobby.signature.title_pattern = "clubgg".to_owned();
    lobby.is_clubgg_lobby = true;
    lobby.likely_table = false;
    backend.candidates.lock().unwrap().push(lobby.clone());
    let mut config = AppConfig {
        auto_arrange: false,
        ..AppConfig::default()
    };
    config.set_disposition(
        lobby.signature,
        clubgg_table_arranger::model::CandidateDisposition::Parked,
    );
    let store = ConfigStore::at(temp_config_path("parked-clubgg-lobby-visible"));
    let handle = spawn_controller(Arc::new(backend), config, store);
    let snapshot = wait_for_snapshot(&handle.snapshots, |snapshot| {
        candidate_mode(snapshot, 8) == WindowMode::Parked
    });

    assert_eq!(snapshot.candidates[0].label, "ClubGG lobby");
    assert!(snapshot.candidates[0].is_clubgg_lobby);
    assert!(snapshot.tables.is_empty());
    assert!(
        snapshot
            .poker_slots
            .iter()
            .all(|slot| slot.occupant.is_none())
    );

    handle.commands.send(ControllerCommand::Shutdown).unwrap();
}

#[test]
fn multiple_lobbies_default_to_park_without_consuming_preserved_columns() {
    let backend = MockBackend::with_tables(1);
    let shared_signature = WindowSignature {
        process_name: "clubgg.exe".to_owned(),
        class_name: "ClubGGLobby".to_owned(),
        title_pattern: "clubgg lobby".to_owned(),
    };
    for id in 8..=10 {
        let mut lobby = candidate(id);
        lobby.label = "ClubGG lobby".to_owned();
        lobby.signature.clone_from(&shared_signature);
        lobby.is_clubgg_lobby = true;
        lobby.likely_table = false;
        backend.candidates.lock().unwrap().push(lobby);
    }
    let store = ConfigStore::at(temp_config_path("multiple-lobbies-no-poker-slots"));
    let config = AppConfig {
        auto_arrange: false,
        ..AppConfig::default()
    };
    let handle = spawn_controller(Arc::new(backend.clone()), config, store);
    let snapshot = wait_for_snapshot(&handle.snapshots, |snapshot| {
        snapshot.candidates.len() == 4
            && snapshot
                .candidates
                .iter()
                .filter(|candidate| candidate.is_clubgg_lobby)
                .all(|candidate| candidate.mode == WindowMode::Parked)
    });

    assert_eq!(snapshot.tables.len(), 1);
    assert!(snapshot.preserve_table_slots);
    assert_eq!(
        snapshot.poker_slots.iter().map(|slot| slot.id.column).max(),
        Some(1)
    );
    assert_eq!(
        snapshot
            .poker_slots
            .iter()
            .filter(|slot| slot.occupant.is_some())
            .count(),
        1
    );

    let moves = backend.moves.lock().unwrap();
    let last_rect = |id| {
        moves
            .iter()
            .rev()
            .find(|(candidate, _)| *candidate == WindowId(id))
            .map(|(_, rect)| *rect)
    };
    assert_eq!(last_rect(8), Some(Rect::new(2512, 924, 240, 180)));
    assert_eq!(last_rect(9), Some(Rect::new(2272, 924, 240, 180)));
    assert_eq!(last_rect(10), Some(Rect::new(2032, 924, 240, 180)));

    handle.commands.send(ControllerCommand::Shutdown).unwrap();
}

#[test]
fn ordinary_windows_default_to_ignore_and_can_use_free_space_or_top_right() {
    let backend = MockBackend::with_tables(2);
    backend
        .candidates
        .lock()
        .unwrap()
        .push(ordinary_candidate(10));
    let store = ConfigStore::at(temp_config_path("ordinary-window-positioning"));
    let config = AppConfig {
        auto_arrange: false,
        ..AppConfig::default()
    };
    let handle = spawn_controller(Arc::new(backend.clone()), config, store.clone());
    let initial = wait_for_snapshot(&handle.snapshots, |snapshot| snapshot.candidates.len() == 3);
    assert_eq!(initial.tables.len(), 2);
    assert_eq!(candidate_mode(&initial, 10), WindowMode::Ignored);
    assert!(backend.moves.lock().unwrap().is_empty());

    handle
        .commands
        .send(ControllerCommand::SetWindowMode {
            id: WindowId(10),
            mode: WindowMode::FreeSpace,
        })
        .unwrap();
    let filled = wait_for_snapshot(&handle.snapshots, |snapshot| {
        candidate_mode(snapshot, 10) == WindowMode::FreeSpace
            && snapshot
                .status_message
                .contains("positioned 1 application window")
    });
    assert_eq!(filled.tables.len(), 2);
    let table_layout = calculate_layout(Rect::new(0, 0, 2752, 1104), 2, 4.0 / 3.0);
    let expected_free = right_side_free_rect(Rect::new(0, 0, 2752, 1104), &table_layout.rectangles);
    assert!(
        backend
            .moves
            .lock()
            .unwrap()
            .iter()
            .any(|(id, rect)| *id == WindowId(10) && *rect == expected_free)
    );
    assert_eq!(
        store
            .load()
            .unwrap()
            .disposition_for(&ordinary_candidate(10).signature),
        Some(clubgg_table_arranger::model::CandidateDisposition::FreeSpace)
    );

    backend.moves.lock().unwrap().clear();
    handle
        .commands
        .send(ControllerCommand::SetWindowMode {
            id: WindowId(10),
            mode: WindowMode::TopRight,
        })
        .unwrap();
    let _ = wait_for_snapshot(&handle.snapshots, |snapshot| {
        candidate_mode(snapshot, 10) == WindowMode::TopRight
            && snapshot
                .status_message
                .contains("positioned 1 application window")
    });
    assert!(
        backend
            .moves
            .lock()
            .unwrap()
            .iter()
            .any(|(id, rect)| *id == WindowId(10) && *rect == Rect::new(1852, 0, 900, 700))
    );
    handle.commands.send(ControllerCommand::Shutdown).unwrap();
    drop(handle);

    let restarted_backend = MockBackend::with_tables(2);
    let mut changed_title = ordinary_candidate(10);
    changed_title.label = "A completely different document title".to_owned();
    changed_title.signature.title_pattern = "a completely different document title".to_owned();
    restarted_backend
        .candidates
        .lock()
        .unwrap()
        .push(changed_title);
    let mut restarted_config = store.load().unwrap();
    restarted_config.auto_arrange = false;
    let restarted = spawn_controller(Arc::new(restarted_backend.clone()), restarted_config, store);
    let restored = wait_for_snapshot(&restarted.snapshots, |snapshot| {
        snapshot.candidates.len() == 3 && candidate_mode(snapshot, 10) == WindowMode::TopRight
    });
    assert_eq!(candidate_mode(&restored, 10), WindowMode::TopRight);
    assert!(restarted_backend.moves.lock().unwrap().is_empty());

    restarted
        .commands
        .send(ControllerCommand::ForceArrange)
        .unwrap();
    let _ = wait_for_snapshot(&restarted.snapshots, |snapshot| {
        snapshot
            .status_message
            .contains("positioned 1 application window")
    });
    assert!(
        restarted_backend
            .moves
            .lock()
            .unwrap()
            .iter()
            .any(|(id, rect)| *id == WindowId(10) && *rect == Rect::new(1852, 0, 900, 700))
    );
    restarted
        .commands
        .send(ControllerCommand::Shutdown)
        .unwrap();
}

#[test]
fn preserved_two_column_boundary_defaults_on_with_zero_or_one_open_table() {
    let work_area = Rect::new(0, 0, 2752, 1104);
    let reserved_layout = calculate_layout(work_area, 2, 4.0 / 3.0);
    let expected_free = right_side_free_rect(work_area, &reserved_layout.rectangles);

    for table_count in [0, 1] {
        let backend = MockBackend::with_tables(table_count);
        let application = ordinary_candidate(10);
        backend.candidates.lock().unwrap().push(application.clone());
        let store = ConfigStore::at(temp_config_path(&format!(
            "reserve-two-slots-{table_count}"
        )));
        let mut config = AppConfig {
            auto_arrange: false,
            ..AppConfig::default()
        };
        config.set_application_disposition(
            application.signature,
            clubgg_table_arranger::model::CandidateDisposition::FreeSpace,
        );
        let handle = spawn_controller(Arc::new(backend.clone()), config, store);
        let initial = wait_for_snapshot(&handle.snapshots, |snapshot| {
            snapshot.tables.len() == table_count
                && candidate_mode(snapshot, 10) == WindowMode::FreeSpace
        });
        assert!(initial.preserve_table_slots);
        assert!(backend.moves.lock().unwrap().is_empty());

        handle
            .commands
            .send(ControllerCommand::ForceArrange)
            .unwrap();
        let _ = wait_for_snapshot(&handle.snapshots, |snapshot| {
            snapshot
                .status_message
                .to_ascii_lowercase()
                .contains("positioned 1 application window")
        });
        assert_eq!(
            backend
                .moves
                .lock()
                .unwrap()
                .iter()
                .rev()
                .find(|(id, _)| *id == WindowId(10))
                .map(|(_, rect)| *rect),
            Some(expected_free)
        );

        handle.commands.send(ControllerCommand::Shutdown).unwrap();
    }
}

#[test]
fn disabling_two_slot_reservation_immediately_reclaims_the_full_empty_display() {
    let backend = MockBackend::with_tables(0);
    let application = ordinary_candidate(10);
    backend.candidates.lock().unwrap().push(application.clone());
    let store = ConfigStore::at(temp_config_path("disable-two-slot-reservation"));
    let mut config = AppConfig {
        auto_arrange: false,
        ..AppConfig::default()
    };
    config.set_application_disposition(
        application.signature,
        clubgg_table_arranger::model::CandidateDisposition::FreeSpace,
    );
    let handle = spawn_controller(Arc::new(backend.clone()), config, store.clone());
    let initial = wait_for_snapshot(&handle.snapshots, |snapshot| {
        snapshot.preserve_table_slots && candidate_mode(snapshot, 10) == WindowMode::FreeSpace
    });
    assert!(initial.preserve_table_slots);

    handle
        .commands
        .send(ControllerCommand::SetPreserveTableSlots(false))
        .unwrap();
    let disabled = wait_for_snapshot(&handle.snapshots, |snapshot| {
        !snapshot.preserve_table_slots
            && snapshot
                .status_message
                .to_ascii_lowercase()
                .contains("positioned 1 application window")
    });
    assert!(!disabled.preserve_table_slots);
    assert_eq!(
        backend
            .moves
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(id, _)| *id == WindowId(10))
            .map(|(_, rect)| *rect),
        Some(Rect::new(0, 0, 2752, 1104))
    );
    assert!(!store.load().unwrap().preserve_table_slots);

    handle.commands.send(ControllerCommand::Shutdown).unwrap();
}

#[test]
fn default_non_poker_mode_is_immediate_persistent_and_overridable() {
    let backend = MockBackend::with_tables(0);
    let application = ordinary_candidate(10);
    backend.candidates.lock().unwrap().push(application.clone());
    let store = ConfigStore::at(temp_config_path("default-non-poker-mode"));
    let config = AppConfig {
        auto_arrange: false,
        default_application_mode: ApplicationDefault::FreeSpace,
        ..AppConfig::default()
    };
    let handle = spawn_controller(Arc::new(backend.clone()), config, store.clone());
    let initial = wait_for_snapshot(&handle.snapshots, |snapshot| {
        snapshot.default_application_mode == ApplicationDefault::FreeSpace
            && candidate_mode(snapshot, 10) == WindowMode::FreeSpace
    });
    assert_eq!(
        initial.default_application_mode,
        ApplicationDefault::FreeSpace
    );

    handle
        .commands
        .send(ControllerCommand::SetDefaultApplicationMode(
            ApplicationDefault::TopRight,
        ))
        .unwrap();
    let top = wait_for_snapshot(&handle.snapshots, |snapshot| {
        snapshot.default_application_mode == ApplicationDefault::TopRight
            && candidate_mode(snapshot, 10) == WindowMode::TopRight
            && snapshot
                .status_message
                .to_ascii_lowercase()
                .contains("positioned 1 application window")
    });
    assert_eq!(top.default_application_mode, ApplicationDefault::TopRight);
    assert!(
        backend
            .moves
            .lock()
            .unwrap()
            .iter()
            .any(|(id, rect)| *id == WindowId(10) && *rect == Rect::new(1852, 0, 900, 700))
    );
    assert_eq!(
        store.load().unwrap().default_application_mode,
        ApplicationDefault::TopRight
    );

    handle
        .commands
        .send(ControllerCommand::SetWindowMode {
            id: WindowId(10),
            mode: WindowMode::Ignored,
        })
        .unwrap();
    let explicit = wait_for_snapshot(&handle.snapshots, |snapshot| {
        candidate_mode(snapshot, 10) == WindowMode::Ignored
    });
    assert_eq!(candidate_mode(&explicit, 10), WindowMode::Ignored);

    handle.commands.send(ControllerCommand::Shutdown).unwrap();
}

#[test]
fn enabling_auto_immediately_reapplies_selected_application_windows() {
    let backend = MockBackend::with_tables(1);
    let application = ordinary_candidate(10);
    backend.candidates.lock().unwrap().push(application.clone());
    let store = ConfigStore::at(temp_config_path("auto-reapplies-ordinary-window"));
    let mut config = AppConfig {
        auto_arrange: false,
        ..AppConfig::default()
    };
    config.set_application_disposition(
        application.signature,
        clubgg_table_arranger::model::CandidateDisposition::FreeSpace,
    );
    let handle = spawn_controller(Arc::new(backend.clone()), config, store);
    let initial = wait_for_snapshot(&handle.snapshots, |snapshot| {
        candidate_mode(snapshot, 10) == WindowMode::FreeSpace
    });
    assert!(!initial.auto_arrange);
    assert!(backend.moves.lock().unwrap().is_empty());

    handle
        .commands
        .send(ControllerCommand::SetAutoArrange(true))
        .unwrap();
    let enabled = wait_for_snapshot(&handle.snapshots, |snapshot| {
        snapshot.auto_arrange
            && snapshot
                .status_message
                .contains("positioned 1 application window")
    });
    assert!(enabled.auto_arrange);
    assert!(
        backend
            .moves
            .lock()
            .unwrap()
            .iter()
            .any(|(id, _)| *id == WindowId(10))
    );
    handle.commands.send(ControllerCommand::Shutdown).unwrap();
}

#[test]
fn reordered_table_slots_survive_controller_restart() {
    let backend = MockBackend::with_tables(4);
    let store = ConfigStore::at(temp_config_path("persisted-table-order"));
    let config = AppConfig {
        auto_arrange: false,
        ..AppConfig::default()
    };
    let handle = spawn_controller(Arc::new(backend.clone()), config, store.clone());
    let _ = wait_for_snapshot(&handle.snapshots, |snapshot| snapshot.tables.len() == 4);
    handle
        .commands
        .send(ControllerCommand::MoveToSlot {
            source: WindowId(4),
            destination: PokerSlotId::club(0, 0),
        })
        .unwrap();
    let reordered = wait_for_snapshot(&handle.snapshots, |snapshot| {
        ids(snapshot) == vec![4, 2, 3, 1]
    });
    assert_eq!(ids(&reordered), vec![4, 2, 3, 1]);
    handle.commands.send(ControllerCommand::Shutdown).unwrap();
    drop(handle);

    let mut restarted_config = store.load().unwrap();
    restarted_config.auto_arrange = false;
    let restarted = spawn_controller(Arc::new(backend), restarted_config, store);
    let restored = wait_for_snapshot(&restarted.snapshots, |snapshot| snapshot.tables.len() == 4);
    assert_eq!(ids(&restored), vec![4, 2, 3, 1]);
    restarted
        .commands
        .send(ControllerCommand::Shutdown)
        .unwrap();
}

fn candidate(number: usize) -> WindowCandidate {
    let label = format!("Synthetic table {number}");
    WindowCandidate {
        id: WindowId(number as u64),
        label: label.clone(),
        process_name: "ClubGG.exe".to_owned(),
        class_name: "SyntheticClubGGTable".to_owned(),
        signature: WindowSignature {
            process_name: "clubgg.exe".to_owned(),
            class_name: "SyntheticClubGGTable".to_owned(),
            title_pattern: label.to_ascii_lowercase(),
        },
        rect: Rect::new(0, 0, 800, 600),
        poker_client: Some(PokerClientKind::ClubGg),
        is_clubgg_lobby: false,
        preferred_aspect_ratio: 4.0 / 3.0,
        likely_table: true,
    }
}

fn ordinary_candidate(number: usize) -> WindowCandidate {
    let label = format!("Document {number}");
    WindowCandidate {
        id: WindowId(number as u64),
        label: label.clone(),
        process_name: "editor.exe".to_owned(),
        class_name: "EditorWindow".to_owned(),
        signature: WindowSignature {
            process_name: "editor.exe".to_owned(),
            class_name: "EditorWindow".to_owned(),
            title_pattern: label.to_ascii_lowercase(),
        },
        rect: Rect::new(100, 100, 900, 700),
        poker_client: None,
        is_clubgg_lobby: false,
        preferred_aspect_ratio: 9.0 / 7.0,
        likely_table: false,
    }
}

fn ldplayer_candidate(number: usize) -> WindowCandidate {
    let label = format!("Pokerrr 2 {number}");
    WindowCandidate {
        id: WindowId(number as u64),
        label: label.clone(),
        process_name: "dnplayer.exe".to_owned(),
        class_name: "LDPlayerMainFrame".to_owned(),
        signature: WindowSignature {
            process_name: "dnplayer.exe".to_owned(),
            class_name: "LDPlayerMainFrame".to_owned(),
            title_pattern: label.to_ascii_lowercase(),
        },
        rect: Rect::new(0, 0, 620, 1104),
        poker_client: Some(PokerClientKind::LdPlayer),
        is_clubgg_lobby: false,
        preferred_aspect_ratio: 9.0 / 16.0,
        likely_table: true,
    }
}

fn ids(snapshot: &UiSnapshot) -> Vec<u64> {
    snapshot.tables.iter().map(|table| table.id.0).collect()
}

fn candidate_ids(snapshot: &UiSnapshot) -> Vec<u64> {
    snapshot
        .candidates
        .iter()
        .map(|candidate| candidate.id.0)
        .collect()
}

fn candidate_mode(snapshot: &UiSnapshot, id: u64) -> WindowMode {
    snapshot
        .candidates
        .iter()
        .find(|candidate| candidate.id == WindowId(id))
        .unwrap()
        .mode
}

fn wait_for_snapshot(
    receiver: &crossbeam_channel::Receiver<Arc<UiSnapshot>>,
    predicate: impl Fn(&UiSnapshot) -> bool,
) -> Arc<UiSnapshot> {
    let deadline = Instant::now() + Duration::from_secs(4);
    while Instant::now() < deadline {
        if let Ok(snapshot) = receiver.recv_timeout(Duration::from_millis(100))
            && predicate(&snapshot)
        {
            return snapshot;
        }
    }
    panic!("timed out waiting for controller snapshot");
}

fn temp_config_path(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "clubgg-table-arranger-{}-{label}-{nonce}.json",
        std::process::id()
    ))
}
