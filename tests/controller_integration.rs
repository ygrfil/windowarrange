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
        BackendError, MonitorInfo, Rect, Size, TableStatus, UiSnapshot, WindowBackend,
        WindowCandidate, WindowId, WindowMode, WindowSignature,
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
        .send(ControllerCommand::Reorder { from: 4, to: 0 })
        .unwrap();
    let reordered = wait_for_snapshot(&handle.snapshots, |snapshot| {
        ids(snapshot) == vec![5, 1, 2, 3, 4]
    });
    assert_eq!(ids(&reordered), vec![5, 1, 2, 3, 4]);

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
    assert_eq!(ids(&closed), vec![5, 1, 2, 4]);

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
fn reserve_two_slots_defaults_on_with_zero_or_one_open_table() {
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
        assert!(initial.reserve_two_slots);
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
        snapshot.reserve_two_slots && candidate_mode(snapshot, 10) == WindowMode::FreeSpace
    });
    assert!(initial.reserve_two_slots);

    handle
        .commands
        .send(ControllerCommand::SetReserveTwoSlots(false))
        .unwrap();
    let disabled = wait_for_snapshot(&handle.snapshots, |snapshot| {
        !snapshot.reserve_two_slots
            && snapshot
                .status_message
                .to_ascii_lowercase()
                .contains("positioned 1 application window")
    });
    assert!(!disabled.reserve_two_slots);
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
    assert!(!store.load().unwrap().reserve_two_slots);

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
        .send(ControllerCommand::Reorder { from: 3, to: 0 })
        .unwrap();
    let reordered = wait_for_snapshot(&handle.snapshots, |snapshot| {
        ids(snapshot) == vec![4, 1, 2, 3]
    });
    assert_eq!(ids(&reordered), vec![4, 1, 2, 3]);
    handle.commands.send(ControllerCommand::Shutdown).unwrap();
    drop(handle);

    let mut restarted_config = store.load().unwrap();
    restarted_config.auto_arrange = false;
    let restarted = spawn_controller(Arc::new(backend), restarted_config, store);
    let restored = wait_for_snapshot(&restarted.snapshots, |snapshot| snapshot.tables.len() == 4);
    assert_eq!(ids(&restored), vec![4, 1, 2, 3]);
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
        is_clubgg: true,
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
        is_clubgg: false,
        likely_table: false,
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
