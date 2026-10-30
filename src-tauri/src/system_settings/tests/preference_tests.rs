use super::*;

#[test]
fn permission_loss_at_final_readback_cannot_be_reported_as_verified() {
    struct FinalLoss {
        state: Mutex<NativeState>,
        reads: AtomicU64,
    }
    impl NativeSettings for FinalLoss {
        fn read(&self) -> Result<NativeState, String> {
            let mut state = self.state.lock().unwrap();
            if self.reads.fetch_add(1, Ordering::SeqCst) == 5 {
                state.values.preferences.remove(&Preference::DockAutohide);
            }
            Ok(state.clone())
        }
        fn write(
            &self,
            _: &NativeState,
            setting: Setting,
            value: SettingValue,
        ) -> Result<(), String> {
            set_value(&mut self.state.lock().unwrap().values, setting, value);
            Ok(())
        }
    }
    let native = Arc::new(FinalLoss {
        state: Mutex::new(broad_state()),
        reads: AtomicU64::new(0),
    });
    let controller = Controller::new(directory("final-permission-loss"), native.clone());
    let saved = controller.capture().unwrap();
    native
        .state
        .lock()
        .unwrap()
        .values
        .preferences
        .insert(Preference::DockAutohide, PreferenceValue::Boolean(true));
    let preview = controller.preview(&saved.id).unwrap();
    let result = controller.confirm(&preview.token).unwrap();
    assert_eq!(result.status, RollbackStatus::PartialFailure);
    assert_eq!(result.outcomes[0].status, OutcomeStatus::ReadbackMismatch);
}

fn broad_state() -> NativeState {
    let mut result = state("speakers");
    for key in Preference::ALL {
        result
            .values
            .preferences
            .insert(key, PreferenceValue::Absent);
    }
    result
}

fn present(key: Preference) -> PreferenceValue {
    match key {
        Preference::DockSize => PreferenceValue::Real(48.0),
        Preference::KeyRepeat | Preference::InitialKeyRepeat => PreferenceValue::Integer(6),
        Preference::MouseSpeed | Preference::TrackpadSpeed => PreferenceValue::Real(0.5),
        _ => PreferenceValue::Boolean(true),
    }
}

#[test]
fn every_allowlisted_preference_roundtrips_through_durable_restore() {
    let directory = directory("all-preferences");
    let (controller, live, writes) = scripted(&directory, broad_state(), None, None);
    let absent = controller.capture().unwrap();
    for key in Preference::ALL {
        live.lock()
            .unwrap()
            .values
            .preferences
            .insert(key, present(key));
    }
    let populated = controller.capture().unwrap();
    let preview = controller.preview(&absent.id).unwrap();
    assert_eq!(preview.changes.len(), Preference::ALL.len());
    assert_eq!(
        controller.confirm(&preview.token).unwrap().status,
        RollbackStatus::Verified
    );
    assert_eq!(live.lock().unwrap().values, broad_state().values);
    let preview = controller.preview(&populated.id).unwrap();
    assert_eq!(
        controller.confirm(&preview.token).unwrap().status,
        RollbackStatus::Verified
    );
    assert_eq!(writes.lock().unwrap().len(), Preference::ALL.len() * 2);
    assert_eq!(controller.history(None, 20).unwrap().entries.len(), 2);
}

#[test]
fn permission_loss_consumes_preview_and_regrant_requires_new_preview() {
    let directory = directory("permission-recheck");
    let (controller, live, writes) = scripted(&directory, broad_state(), None, None);
    let baseline = controller.capture().unwrap();
    live.lock()
        .unwrap()
        .values
        .preferences
        .insert(Preference::DockAutohide, present(Preference::DockAutohide));
    let preview = controller.preview(&baseline.id).unwrap();
    // Denial/managed restriction removes the preference from the writable adapter inventory.
    live.lock()
        .unwrap()
        .values
        .preferences
        .remove(&Preference::DockAutohide);
    assert!(controller
        .confirm(&preview.token)
        .unwrap_err()
        .contains("stale"));
    assert!(controller
        .preview(&baseline.id)
        .unwrap_err()
        .contains("unavailable"));
    assert!(writes.lock().unwrap().is_empty());
    live.lock()
        .unwrap()
        .values
        .preferences
        .insert(Preference::DockAutohide, present(Preference::DockAutohide));
    assert!(controller.confirm(&preview.token).is_err());
    let refreshed = controller.preview(&baseline.id).unwrap();
    assert_eq!(
        controller.confirm(&refreshed.token).unwrap().status,
        RollbackStatus::Verified
    );
}

#[test]
fn preference_write_denial_reports_partial_and_never_replays() {
    let directory = directory("preference-denied");
    let (controller, live, writes) = scripted(
        &directory,
        broad_state(),
        Some(Setting::Preference(Preference::DockMagnification)),
        None,
    );
    let baseline = controller.capture().unwrap();
    for key in Preference::ALL {
        live.lock()
            .unwrap()
            .values
            .preferences
            .insert(key, present(key));
    }
    let preview = controller.preview(&baseline.id).unwrap();
    let result = controller.confirm(&preview.token).unwrap();
    assert_eq!(result.status, RollbackStatus::PartialFailure);
    assert_eq!(result.outcomes[0].status, OutcomeStatus::Verified);
    assert_eq!(result.outcomes[1].status, OutcomeStatus::WriteFailed);
    assert!(result.outcomes[2..]
        .iter()
        .all(|v| v.status == OutcomeStatus::Pending));
    assert_eq!(writes.lock().unwrap().len(), 2);
    controller.history(None, 20).unwrap();
    assert_eq!(writes.lock().unwrap().len(), 2);
}

#[test]
fn saved_preference_mismatch_is_not_reported_as_verified() {
    let directory = directory("preference-readback");
    let (controller, live, writes) = scripted(
        &directory,
        broad_state(),
        None,
        Some(Setting::Preference(Preference::DockAutohide)),
    );
    let baseline = controller.capture().unwrap();
    for key in Preference::ALL {
        live.lock()
            .unwrap()
            .values
            .preferences
            .insert(key, present(key));
    }
    let preview = controller.preview(&baseline.id).unwrap();
    let result = controller.confirm(&preview.token).unwrap();
    assert_eq!(result.status, RollbackStatus::PartialFailure);
    assert_eq!(result.outcomes[0].status, OutcomeStatus::ReadbackMismatch);
    assert_eq!(writes.lock().unwrap().len(), 1);
}

#[test]
fn preference_drift_and_os_update_each_reject_stale_confirmation() {
    for os_change in [false, true] {
        let directory = directory("preference-drift");
        let (controller, live, writes) = scripted(&directory, broad_state(), None, None);
        let baseline = controller.capture().unwrap();
        let preview = controller.preview(&baseline.id).unwrap();
        if os_change {
            live.lock().unwrap().identity.os_version = "16.0".into();
        } else {
            live.lock()
                .unwrap()
                .values
                .preferences
                .insert(Preference::NaturalScroll, PreferenceValue::Boolean(true));
        }
        assert!(controller
            .confirm(&preview.token)
            .unwrap_err()
            .contains("stale"));
        assert!(writes.lock().unwrap().is_empty());
        assert_eq!(controller.history(None, 20).unwrap().entries.len(), 1);
    }
}

#[test]
fn old_audio_snapshot_restores_audio_without_touching_new_preferences() {
    let directory = directory("old-audio");
    let (controller, live, writes) = scripted(&directory, state("speakers"), None, None);
    let baseline = controller.capture().unwrap();
    let path = controller
        .root()
        .join(SNAPSHOTS)
        .join(format!("{}.json", baseline.id));
    // Reproduce an existing v1 snapshot before either additive field existed.
    let mut old: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    old.as_object_mut().unwrap().remove("coverage");
    old["values"].as_object_mut().unwrap().remove("preferences");
    fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
    live.lock()
        .unwrap()
        .values
        .preferences
        .insert(Preference::MouseSpeed, PreferenceValue::Real(1.25));
    live.lock().unwrap().values.output_volume = Some(0.8);
    let preview = controller.preview(&baseline.id).unwrap();
    assert_eq!(
        controller.confirm(&preview.token).unwrap().status,
        RollbackStatus::Verified
    );
    assert_eq!(*writes.lock().unwrap(), vec![Setting::OutputVolume]);
    assert_eq!(
        live.lock().unwrap().values.preferences[&Preference::MouseSpeed],
        PreferenceValue::Real(1.25)
    );
    assert!(controller
        .inspect(&baseline.id)
        .unwrap()
        .coverage
        .is_empty());
}

#[test]
fn preference_only_baseline_does_not_require_settable_audio() {
    let directory = directory("no-audio");
    let mut state = broad_state();
    state.values.output_volume = None;
    state.values.output_muted = None;
    let (controller, live, _) = scripted(&directory, state, None, None);
    let baseline = controller.capture().unwrap();
    live.lock()
        .unwrap()
        .values
        .preferences
        .insert(Preference::KeyRepeat, PreferenceValue::Integer(8));
    let preview = controller.preview(&baseline.id).unwrap();
    assert_eq!(
        controller.confirm(&preview.token).unwrap().status,
        RollbackStatus::Verified
    );
}

#[test]
fn validation_rejects_wrong_types_unbounded_numbers_and_unknown_keys() {
    assert!(Preference::KeyRepeat
        .validate(PreferenceValue::Boolean(true))
        .is_err());
    assert!(Preference::KeyRepeat
        .validate(PreferenceValue::Integer(i64::MAX))
        .is_err());
    assert!(Preference::MouseSpeed
        .validate(PreferenceValue::Real(f64::NAN))
        .is_err());
    assert!(Preference::MouseSpeed
        .validate(PreferenceValue::Real(f64::INFINITY))
        .is_err());
    assert!(Preference::DockAutohide
        .validate(PreferenceValue::Integer(1))
        .is_err());
    assert!(serde_json::from_str::<Preference>("\"arbitrary_key\"").is_err());
    assert!(settings_url("https://example.com").is_err());
    assert!(settings_url("; open -a Terminal").is_err());
}

#[test]
fn inventory_has_unique_ids_and_never_offers_a_fake_privileged_grant() {
    let inventory = capabilities::coverage(&broad_state().values);
    let ids: std::collections::BTreeSet<_> = inventory.iter().map(|item| &item.id).collect();
    assert_eq!(inventory.len(), 20);
    assert_eq!(ids.len(), inventory.len());
    assert_eq!(
        inventory
            .iter()
            .filter(|item| item.status == CapabilityStatus::Supported)
            .count(),
        14
    );
    for id in [
        "display_sleep",
        "system_sleep",
        "brightness",
        "stage_manager",
    ] {
        let item = inventory.iter().find(|item| item.id == id).unwrap();
        assert_eq!(item.status, CapabilityStatus::Unavailable);
        assert!(settings_url(item.pane.as_ref().unwrap()).is_ok());
    }
}
