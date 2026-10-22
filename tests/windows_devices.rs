#![cfg(windows)]
//! Opt-in tests for a Windows PC with a physical microphone and VB-CABLE.
//! Run serially, with other MicLift instances closed. Never install/remove a driver.
use miclift::{
    audio::Engine,
    config::{Config, VoiceSettings},
    platform,
};
use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

fn wait_for(engine: &Engine, predicate: impl Fn(&miclift::audio::Status) -> bool) {
    let start = Instant::now();
    loop {
        let state = engine.status.lock().unwrap().clone();
        if predicate(&state) {
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "{}",
            state.message
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
#[ignore = "requires a live Windows microphone and VB-CABLE; sends microphone audio to cable"]
fn live_gain_changes_do_not_restart_streams_and_stop_releases_capture() {
    let engine = Engine::new();
    engine.start(&Config::default());
    wait_for(&engine, |state| state.running);
    engine.parameters.store(&VoiceSettings {
        boost: 500.0,
        ..Default::default()
    });
    engine.parameters.muted.store(true, Ordering::Relaxed);
    std::thread::sleep(Duration::from_millis(200));
    engine.parameters.muted.store(false, Ordering::Relaxed);
    assert_eq!(engine.telemetry.sessions_started.load(Ordering::Relaxed), 1);
    assert!(engine.telemetry.frames.load(Ordering::Relaxed) > 0);
    engine.stop();
    wait_for(&engine, |state| !state.running && !state.requested);
    let frames = engine.telemetry.frames.load(Ordering::Relaxed);
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(engine.telemetry.frames.load(Ordering::Relaxed), frames);
}

#[test]
#[ignore = "requires a live Windows microphone and VB-CABLE; sends microphone audio to cable"]
fn unavailable_listen_output_does_not_prevent_microphone_routing() {
    let engine = Engine::new();
    engine.parameters.monitor.store(true, Ordering::Relaxed);
    engine.start(&Config {
        monitor_id: "miclift-test-unavailable-output".into(),
        ..Default::default()
    });
    wait_for(&engine, |state| state.running);
    assert!(!engine.parameters.monitor.load(Ordering::Relaxed));
    assert!(
        engine
            .status
            .lock()
            .unwrap()
            .message
            .contains("unavailable")
    );
}

#[test]
#[ignore = "requires a live Windows microphone and VB-CABLE; sends microphone audio to cable"]
fn failed_device_start_can_recover_without_a_process_restart() {
    let engine = Engine::new();
    engine.start(&Config {
        input_id: "miclift-test-unavailable-input".into(),
        ..Default::default()
    });
    let start = Instant::now();
    while engine.telemetry.session_errors.load(Ordering::Relaxed) == 0 {
        assert!(start.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!engine.status.lock().unwrap().running);
    engine.start(&Config::default());
    wait_for(&engine, |state| state.running);
}

#[test]
#[ignore = "temporarily tests HKCU startup registration, then restores the original value"]
fn startup_registration_roundtrip_restores_the_original_value() {
    use winreg::{RegKey, RegValue, enums::HKEY_CURRENT_USER};
    struct Restore {
        key: RegKey,
        value: Option<RegValue<'static>>,
    }
    impl Drop for Restore {
        fn drop(&mut self) {
            if let Some(value) = &self.value {
                self.key.set_raw_value("MicLift", value).unwrap();
            } else {
                let _ = self.key.delete_value("MicLift");
            }
        }
    }
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Run")
        .unwrap();
    let value = key.get_raw_value("MicLift").ok();
    let restore = Restore { key, value };
    platform::set_autostart(true).unwrap();
    assert!(platform::autostart_enabled());
    let command: String = restore.key.get_value("MicLift").unwrap();
    assert_eq!(
        command,
        format!(
            "\"{}\" --autostart",
            std::env::current_exe().unwrap().display()
        )
    );
    platform::set_autostart(false).unwrap();
    assert!(!platform::autostart_enabled());
}
