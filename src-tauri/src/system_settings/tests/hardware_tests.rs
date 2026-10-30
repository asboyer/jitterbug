use super::*;

fn hardware_state() -> NativeState {
    let mut initial = state("speakers");
    initial.values.hardware = serde_json::from_value(serde_json::json!({
        "0":{"kind":"audio","uid":"synthetic-output","rate":48000.0,"streams":[{
            "starting_channel":1,"terminal_type":1,"format":{"sample_rate":48000.0,"format_id":1819304813,"flags":1,"bytes_per_packet":8,"frames_per_packet":1,"bytes_per_frame":8,"channels":2,"bits_per_channel":32,"reserved":0}
        }]},
        "100":{"kind":"power","source":"AC Power","key":"sleep","minutes":12},
        "101":{"kind":"power","source":"AC Power","key":"displaysleep","minutes":5},
        "102":{"kind":"power","source":"Battery Power","key":"sleep","minutes":0},
        "105":{"kind":"power","source":"UPS Power","key":"displaysleep","minutes":0}
    })).unwrap();
    initial.values.validate().unwrap();
    initial
}

#[test]
fn full_asbd_roundtrips_and_stream_identity_drift_is_rejected() {
    let initial = hardware_state();
    let encoded = serde_json::to_value(&initial.values).unwrap();
    let roundtrip: SystemValues = serde_json::from_value(encoded.clone()).unwrap();
    assert_eq!(roundtrip, initial.values);
    let directory = directory("asbd-identity");
    let (controller, live, writes) = scripted(&directory, initial, None, None);
    let baseline = controller
        .capture_selected(Some(&["Audio hardware".into()]))
        .unwrap();
    let mut changed = encoded.clone();
    changed["hardware"]["0"]["streams"][0]["format"]["bits_per_channel"] = 24.into();
    live.lock().unwrap().values = serde_json::from_value(changed.clone()).unwrap();
    let preview = controller.preview(&baseline.id).unwrap();
    assert!(preview.changes[0].before.contains("bits/channel=24"));
    assert_eq!(
        controller.confirm(&preview.token).unwrap().status,
        RollbackStatus::Verified
    );
    assert_eq!(*writes.lock().unwrap(), vec![Setting::Hardware(0)]);
    for (path, value) in [("starting_channel", 2), ("terminal_type", 2)] {
        let mut drift = encoded.clone();
        drift["hardware"]["0"]["streams"][0][path] = value.into();
        live.lock().unwrap().values = serde_json::from_value(drift).unwrap();
        assert!(controller.preview(&baseline.id).is_err());
    }
    changed["hardware"]["0"]["uid"] = "replacement-output".into();
    live.lock().unwrap().values = serde_json::from_value(changed).unwrap();
    assert!(controller.preview(&baseline.id).is_err());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn category_selection_excludes_permission_needed_power_and_legacy_does_not_restore_hardware() {
    let directory = directory("hardware-scope");
    let (controller, live, writes) = scripted(&directory, hardware_state(), None, None);
    let saved = controller
        .capture_selected(Some(&["Audio".into()]))
        .unwrap();
    assert!(saved.values.as_ref().unwrap().hardware.is_empty());
    assert!(saved
        .coverage
        .iter()
        .filter(|c| c.id.starts_with("hardware_"))
        .all(|c| !c.included));
    {
        let mut state = live.lock().unwrap();
        state.values.output_volume = Some(0.9);
        if let HardwareValue::Power { minutes, .. } = state.values.hardware.get_mut(&100).unwrap() {
            *minutes = 90;
        }
    }
    let hardware_before = live.lock().unwrap().values.hardware.clone();
    let preview = controller.preview(&saved.id).unwrap();
    assert_eq!(
        controller.confirm(&preview.token).unwrap().status,
        RollbackStatus::Verified
    );
    assert_eq!(live.lock().unwrap().values.hardware, hardware_before);
    assert_eq!(*writes.lock().unwrap(), vec![Setting::OutputVolume]);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn power_denial_is_partial_durable_and_does_not_attempt_later_profiles() {
    let directory = directory("power-partial");
    let (controller, live, writes) = scripted(
        &directory,
        hardware_state(),
        Some(Setting::Hardware(101)),
        None,
    );
    let baseline = controller
        .capture_selected(Some(&["Power profiles".into()]))
        .unwrap();
    for value in live.lock().unwrap().values.hardware.values_mut() {
        if let HardwareValue::Power { minutes, .. } = value {
            *minutes = 99;
        }
    }
    let preview = controller.preview(&baseline.id).unwrap();
    let result = controller.confirm(&preview.token).unwrap();
    assert_eq!(result.status, RollbackStatus::PartialFailure);
    assert_eq!(result.outcomes[0].status, OutcomeStatus::Verified);
    assert_eq!(result.outcomes[1].status, OutcomeStatus::WriteFailed);
    assert!(result.outcomes[2..]
        .iter()
        .all(|v| v.status == OutcomeStatus::Pending));
    assert_eq!(
        *writes.lock().unwrap(),
        vec![Setting::Hardware(100), Setting::Hardware(101)]
    );
    let stored: JournalFile = serde_json::from_slice(
        &fs::read(
            directory
                .join(DIRECTORY)
                .join(JOURNALS)
                .join(format!("{}.json", result.journal_id.unwrap())),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(stored.status, JournalStatus::PartialFailure);
    assert_eq!(controller.history(None, 20).unwrap().entries.len(), 1);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn invalid_or_injected_hardware_values_cannot_become_targets() {
    let values = hardware_state().values;
    for source in ["AC Power;id", "-a", "Unknown"] {
        let value = HardwareValue::Power {
            source: source.into(),
            key: "sleep".into(),
            minutes: 0,
        };
        assert!(value.validate(100).is_err());
    }
    assert!(HardwareValue::Power {
        source: "AC Power".into(),
        key: "sleep".into(),
        minutes: u32::MAX
    }
    .validate(100)
    .is_err());
    assert!(values.hardware[&100].validate(102).is_err());
    let mut duplicate = values.hardware[&0].clone();
    if let HardwareValue::Audio { streams, .. } = &mut duplicate {
        streams.push(streams[0].clone());
    }
    assert!(duplicate.validate(0).is_err());
}
