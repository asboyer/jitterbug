use super::*;

#[test]
fn text_and_network_storage_preferences_have_exact_boolean_scopes() {
    for (id, domain, key) in [
        (
            "automatic_spelling",
            "NSGlobalDomain",
            "NSAutomaticSpellingCorrectionEnabled",
        ),
        (
            "automatic_capitalization",
            "NSGlobalDomain",
            "NSAutomaticCapitalizationEnabled",
        ),
        (
            "automatic_period",
            "NSGlobalDomain",
            "NSAutomaticPeriodSubstitutionEnabled",
        ),
        (
            "automatic_dash",
            "NSGlobalDomain",
            "NSAutomaticDashSubstitutionEnabled",
        ),
        (
            "automatic_quote",
            "NSGlobalDomain",
            "NSAutomaticQuoteSubstitutionEnabled",
        ),
        (
            "ds_store_network",
            "com.apple.desktopservices",
            "DSDontWriteNetworkStores",
        ),
        (
            "ds_store_usb",
            "com.apple.desktopservices",
            "DSDontWriteUSBStores",
        ),
    ] {
        let preference: Preference = serde_json::from_value(serde_json::json!(id)).unwrap();
        assert!(Preference::ALL.contains(&preference));
        assert_eq!(
            (preference.descriptor().0, preference.descriptor().1),
            (domain, key)
        );
        assert!(preference.validate(PreferenceValue::Absent).is_ok());
        assert!(preference.validate(PreferenceValue::Boolean(false)).is_ok());
        assert!(preference.validate(PreferenceValue::Integer(1)).is_err());
    }
}

#[test]
fn finder_storage_can_be_captured_alone_without_other_settings() {
    let (controller, _, writes) = scripted(&directory("finder-only"), broad_state(), None, None);
    let saved = controller
        .capture_selected(Some(&["Finder storage".into()]))
        .unwrap();
    let values = saved.values.unwrap();
    assert_eq!(values.preferences.len(), 2);
    assert!(values.output_volume.is_none());
    assert!(values.output_muted.is_none());
    assert!(writes.lock().unwrap().is_empty());
    for item in saved
        .coverage
        .iter()
        .filter(|v| v.status != CapabilityStatus::Supported)
    {
        assert!(
            !item.detail.starts_with("Not selected for this capture."),
            "{} is not selectable",
            item.id
        );
    }
}

#[test]
fn wishlist_entries_are_manual_and_never_selectable() {
    let inventory = capabilities::coverage(&broad_state().values);
    for id in [
        "audio_format",
        "display_mode",
        "system_sleep",
        "display_sleep",
        "low_power",
        "high_power",
        "login_items",
        "background_items",
        "time_machine",
    ] {
        let item = inventory.iter().find(|v| v.id == id).unwrap();
        assert_eq!(item.status, CapabilityStatus::Unavailable, "{id}");
        assert!(!item.included);
        assert!(item.detail.contains("executor") || item.detail.contains("adapter"));
    }
    for id in [
        "microphone_privacy",
        "spotlight_exclusions",
        "developer_access",
    ] {
        let item = inventory.iter().find(|v| v.id == id).expect(id);
        assert!(!item.included);
        assert_eq!(serde_json::to_value(item.status).unwrap(), "manual");
        assert!(settings_url(item.pane.as_deref().unwrap()).is_ok());
    }
}

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
fn original_fourteen_setting_snapshot_restores_without_touching_new_preferences() {
    let root = directory("original-fourteen");
    let mut old = broad_state();
    old.values.preferences.retain(|key, _| {
        matches!(
            key,
            Preference::DockAutohide
                | Preference::DockMagnification
                | Preference::DockAnimate
                | Preference::DockMinimizeToApplication
                | Preference::DockShowRecents
                | Preference::DockSize
                | Preference::KeyRepeat
                | Preference::InitialKeyRepeat
                | Preference::PressAndHold
                | Preference::NaturalScroll
                | Preference::MouseSpeed
                | Preference::TrackpadSpeed
        )
    });
    assert_eq!(old.values.preferences.len(), 12);
    let old_keys = old.values.preferences.clone();
    let (controller, live, writes) = scripted(&root, old, None, None);
    let saved = controller.capture().unwrap();
    let path = controller
        .root()
        .join(SNAPSHOTS)
        .join(format!("{}.json", saved.id));
    let mut file: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    file["coverage"].as_array_mut().unwrap().retain(|item| {
        matches!(
            item["id"].as_str(),
            Some(
                "output_volume"
                    | "output_muted"
                    | "stage_manager"
                    | "reduce_motion"
                    | "reduce_transparency"
                    | "display_sleep"
                    | "system_sleep"
                    | "brightness"
            )
        ) || serde_json::from_value::<Preference>(item["id"].clone())
            .is_ok_and(|key| old_keys.contains_key(&key))
    });
    assert_eq!(file["coverage"].as_array().unwrap().len(), 20);
    fs::write(&path, serde_json::to_vec(&file).unwrap()).unwrap();
    let legacy_bytes = fs::read(&path).unwrap();
    {
        let mut current = live.lock().unwrap();
        current.values.output_volume = Some(0.8);
        current.values.output_muted = Some(true);
        for key in Preference::ALL {
            current.values.preferences.insert(key, present(key));
        }
    }
    let latest = controller.capture().unwrap();
    let preview = controller.preview(&saved.id).unwrap();
    assert_eq!(preview.changes.len(), 14);
    assert_eq!(
        controller.confirm(&preview.token).unwrap().status,
        RollbackStatus::Verified
    );
    assert_eq!(writes.lock().unwrap().len(), 14);
    for key in Preference::ALL {
        let value = live.lock().unwrap().values.preferences[&key];
        assert_eq!(
            value,
            if old_keys.contains_key(&key) {
                PreferenceValue::Absent
            } else {
                present(key)
            }
        );
    }
    assert_eq!(fs::read(path).unwrap(), legacy_bytes);
    assert!(controller.inspect(&saved.id).unwrap().compatible);
    let page = controller.history(None, 1).unwrap();
    assert_eq!(page.entries[0].id, latest.id);
    assert_eq!(
        controller
            .history(page.next_cursor.as_deref(), 1)
            .unwrap()
            .entries[0]
            .id,
        saved.id
    );
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
    assert_eq!(inventory.len(), 2 + Preference::ALL.len() + 14);
    assert_eq!(ids.len(), inventory.len());
    assert!(inventory
        .iter()
        .any(|item| item.id == "output_volume" && item.status == CapabilityStatus::Supported));
    for id in ["display_sleep", "system_sleep", "brightness"] {
        let item = inventory.iter().find(|item| item.id == id).unwrap();
        assert_eq!(item.status, CapabilityStatus::Unavailable);
        assert!(settings_url(item.pane.as_ref().unwrap()).is_ok());
    }
}
