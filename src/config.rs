use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub const CONFIG_VERSION: u32 = 2;
pub const EQ_PRESETS: [&str; 5] = ["Flat", "Voice", "Warm", "Podcast", "Bright"];

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EqBand {
    pub frequency: f32,
    pub gain_db: f32,
    pub q: f32,
}
impl Default for EqBand {
    fn default() -> Self {
        Self {
            frequency: 1000.0,
            gain_db: 0.0,
            q: 0.707,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceSettings {
    pub boost: f32,
    pub eq_enabled: bool,
    pub bands: [EqBand; 5],
    pub high_pass: bool,
    pub gate: bool,
    pub gate_threshold_db: f32,
    pub compressor: bool,
    pub compressor_threshold_db: f32,
    pub compressor_ratio: f32,
    pub limiter: bool,
}
impl Default for VoiceSettings {
    fn default() -> Self {
        Self {
            boost: 100.0,
            eq_enabled: true,
            bands: [100.0, 300.0, 1000.0, 3000.0, 8000.0].map(|frequency| EqBand {
                frequency,
                ..EqBand::default()
            }),
            high_pass: false,
            gate: false,
            gate_threshold_db: -48.0,
            compressor: false,
            compressor_threshold_db: -18.0,
            compressor_ratio: 3.0,
            limiter: true,
        }
    }
}
fn bounded(value: f32, min: f32, max: f32, default: f32) -> f32 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        default
    }
}
impl VoiceSettings {
    pub fn sanitize(&mut self) {
        self.boost = bounded(self.boost, 0.0, 500.0, 100.0);
        for band in &mut self.bands {
            band.frequency = bounded(band.frequency, 20.0, 20000.0, 1000.0);
            band.gain_db = bounded(band.gain_db, -12.0, 12.0, 0.0);
            band.q = bounded(band.q, 0.1, 12.0, 0.707);
        }
        self.gate_threshold_db = bounded(self.gate_threshold_db, -80.0, -12.0, -48.0);
        self.compressor_threshold_db = bounded(self.compressor_threshold_db, -40.0, -3.0, -18.0);
        self.compressor_ratio = bounded(self.compressor_ratio, 1.0, 10.0, 3.0);
    }
    pub fn preset(&mut self, name: &str) {
        let gains = match name {
            "Flat" => [0.0; 5],
            "Voice" => [-3.0, -2.0, 1.0, 3.0, 1.0],
            "Warm" => [3.0, 1.5, 0.0, -1.0, -2.0],
            "Podcast" => [-2.0, -2.5, 0.5, 2.5, 1.5],
            "Bright" => [-1.0, -1.5, 0.0, 2.0, 3.5],
            _ => return,
        };
        self.eq_enabled = true;
        self.bands = Self::default().bands;
        for (band, gain) in self.bands.iter_mut().zip(gains) {
            band.gain_db = gain;
        }
    }
    pub fn eq_preset_index(&self) -> i32 {
        EQ_PRESETS
            .iter()
            .position(|name| {
                let mut candidate = *self;
                candidate.preset(name);
                self.eq_enabled && self.bands == candidate.bands
            })
            .map_or(-1, |index| index as i32)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub version: u32,
    pub input_id: String,
    pub cable_id: String,
    pub monitor_id: String,
    pub monitor_volume: f32,
    pub voice: VoiceSettings,
    pub profiles: BTreeMap<String, VoiceSettings>,
    pub saved_profiles: BTreeMap<String, VoiceSettings>,
    pub active_profile: String,
    pub default_profile: String,
    pub reduce_motion: bool,
    pub start_with_windows: bool,
    pub start_minimized: bool,
    pub start_audio: bool,
    pub close_to_tray: bool,
    pub setup_complete: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            input_id: String::new(),
            cable_id: String::new(),
            monitor_id: String::new(),
            monitor_volume: 0.5,
            voice: VoiceSettings::default(),
            profiles: BTreeMap::new(),
            saved_profiles: BTreeMap::new(),
            active_profile: String::new(),
            default_profile: String::new(),
            reduce_motion: false,
            start_with_windows: false,
            start_minimized: false,
            start_audio: false,
            close_to_tray: true,
            setup_complete: false,
        }
    }
}
impl Config {
    pub fn path() -> Result<PathBuf> {
        Ok(
            PathBuf::from(std::env::var_os("APPDATA").context("APPDATA is unavailable")?)
                .join("MicLift")
                .join("settings.json"),
        )
    }
    pub fn load() -> Result<Self> {
        let path = Self::path()?;
        Self::load_path(&path)
    }
    fn load_path(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let mut config: Self = serde_json::from_slice(&std::fs::read(path)?)
            .context("Settings are invalid; original file has been preserved")?;
        if !(1..=CONFIG_VERSION).contains(&config.version) {
            bail!(
                "Unsupported settings version {}; settings preserved",
                config.version
            );
        }
        config.version = CONFIG_VERSION;
        config.voice.sanitize();
        config.monitor_volume = bounded(config.monitor_volume, 0.0, 1.0, 0.5);
        for voice in config.profiles.values_mut() {
            voice.sanitize();
        }
        for voice in config.saved_profiles.values_mut() {
            voice.sanitize();
        }
        config.ensure_profiles();
        Ok(config)
    }
    pub fn save(&self) -> Result<()> {
        let path = Self::path()?;
        self.save_path(&path)
    }
    fn save_path(&self, path: &Path) -> Result<()> {
        std::fs::create_dir_all(path.parent().context("Settings directory missing")?)?;
        let tmp = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(self)?;
        use std::io::Write;
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        #[cfg(windows)]
        {
            use windows::{
                Win32::Storage::FileSystem::{
                    MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
                },
                core::HSTRING,
            };
            unsafe {
                MoveFileExW(
                    &HSTRING::from(tmp.as_os_str()),
                    &HSTRING::from(path.as_os_str()),
                    MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
                )?;
            }
        }
        #[cfg(not(windows))]
        std::fs::rename(tmp, path)?;
        Ok(())
    }
    pub fn select_input(&mut self, id: String) {
        if !self.input_id.is_empty() {
            self.profiles.insert(self.input_id.clone(), self.voice);
        }
        self.input_id = id;
        self.voice = self
            .profiles
            .get(&self.input_id)
            .copied()
            .unwrap_or_default();
    }
    pub fn ensure_profiles(&mut self) {
        if self.saved_profiles.is_empty() {
            self.saved_profiles.insert("My setup".into(), self.voice);
            self.active_profile = "My setup".into();
        }
        if !self.saved_profiles.contains_key(&self.active_profile) {
            self.active_profile = self.saved_profiles.keys().next().unwrap().clone();
        }
        if !self.saved_profiles.contains_key(&self.default_profile) {
            self.default_profile.clear();
        }
    }
    pub fn profile_modified(&self) -> bool {
        self.saved_profiles.get(&self.active_profile) != Some(&self.voice)
    }
    pub fn save_profile_as(&mut self, name: &str) -> Result<()> {
        let name = name.trim();
        anyhow::ensure!(!name.is_empty(), "Enter a profile name first.");
        anyhow::ensure!(
            name.chars().count() <= 40 && !name.chars().any(char::is_control),
            "Use a profile name of 1–40 characters without control characters."
        );
        anyhow::ensure!(
            !self
                .saved_profiles
                .keys()
                .any(|existing| existing.to_lowercase() == name.to_lowercase()),
            "A profile with this name already exists. Use Save changes to update it."
        );
        self.saved_profiles.insert(name.into(), self.voice);
        self.active_profile = name.into();
        Ok(())
    }
    pub fn save_active_profile(&mut self) -> Result<()> {
        let saved = self
            .saved_profiles
            .get_mut(&self.active_profile)
            .context("Select a profile first.")?;
        *saved = self.voice;
        Ok(())
    }
    pub fn select_profile(&mut self, name: &str) -> Result<()> {
        self.voice = *self
            .saved_profiles
            .get(name)
            .context("Profile no longer exists.")?;
        self.active_profile = name.into();
        Ok(())
    }
    pub fn make_profile_default(&mut self) {
        self.default_profile = self.active_profile.clone();
    }
    pub fn apply_startup_profile(&mut self) {
        if let Some(voice) = self.saved_profiles.get(&self.default_profile) {
            self.voice = *voice;
            self.active_profile = self.default_profile.clone();
        }
    }
    pub fn delete_active_profile(&mut self) -> Result<()> {
        anyhow::ensure!(
            self.saved_profiles.len() > 1,
            "Keep at least one saved profile."
        );
        self.saved_profiles.remove(&self.active_profile);
        if self.default_profile == self.active_profile {
            self.default_profile.clear();
        }
        let next = self.saved_profiles.keys().next().unwrap().clone();
        self.select_profile(&next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profiles_restore_all_voice_settings_and_default_on_launch() {
        let mut config = Config::default();
        config.ensure_profiles();
        config.voice.boost = 275.0;
        config.voice.preset("Podcast");
        config.voice.high_pass = true;
        config.voice.gate = true;
        config.voice.gate_threshold_db = -42.0;
        config.voice.compressor = true;
        config.voice.compressor_ratio = 4.0;
        config.save_profile_as("Podcast night").unwrap();
        let podcast = config.voice;
        config.make_profile_default();
        config.select_profile("My setup").unwrap();
        assert_eq!(config.voice, VoiceSettings::default());
        config.voice.boost = 400.0;
        assert!(config.profile_modified());
        config.apply_startup_profile();
        assert_eq!(config.voice, podcast);
        assert_eq!(config.active_profile, "Podcast night");
        assert!(!config.profile_modified());
        config.voice.boost = 300.0;
        config.save_active_profile().unwrap();
        config.select_profile("My setup").unwrap();
        config.apply_startup_profile();
        assert_eq!(config.voice.boost, 300.0);
        config.delete_active_profile().unwrap();
        assert!(config.default_profile.is_empty());
        assert_eq!(config.active_profile, "My setup");
        assert!(config.delete_active_profile().is_err());
    }
    #[test]
    fn profile_names_are_validated_without_overwriting_saved_settings() {
        let mut config = Config::default();
        config.ensure_profiles();
        for name in [
            "",
            " ",
            "MY SETUP",
            "My setup",
            "bad\nname",
            &"x".repeat(41),
        ] {
            assert!(config.save_profile_as(name).is_err());
        }
        config.save_profile_as("  Gaming  ").unwrap();
        assert_eq!(config.active_profile, "Gaming");
        assert_eq!(config.saved_profiles.len(), 2);
    }
    #[test]
    fn eq_presets_preserve_boost_and_processing() {
        let original = VoiceSettings {
            boost: 450.0,
            high_pass: true,
            gate: true,
            gate_threshold_db: -33.0,
            compressor: true,
            compressor_threshold_db: -12.0,
            compressor_ratio: 5.0,
            limiter: false,
            ..Default::default()
        };
        for (index, name) in EQ_PRESETS.iter().enumerate() {
            let mut voice = original;
            voice.preset(name);
            assert_eq!(voice.eq_preset_index(), index as i32);
            let mut without_eq = voice;
            without_eq.bands = original.bands;
            assert_eq!(without_eq, original);
        }
    }
    #[test]
    fn atomic_settings_roundtrip_and_invalid_files_are_preserved() {
        let root = std::env::temp_dir().join(format!(
            "miclift-config-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = root.join("settings.json");
        let mut config = Config::load_path(&path).unwrap();
        config.voice.boost = 425.0;
        config.ensure_profiles();
        config.save_profile_as("Saved boost").unwrap();
        config.make_profile_default();
        config.save_path(&path).unwrap();
        assert_eq!(Config::load_path(&path).unwrap().voice.boost, 425.0);
        let restored = Config::load_path(&path).unwrap();
        assert_eq!(restored.saved_profiles["Saved boost"].boost, 425.0);
        assert_eq!(restored.default_profile, "Saved boost");
        config.voice.boost = 250.0;
        config.save_path(&path).unwrap();
        assert_eq!(Config::load_path(&path).unwrap().voice.boost, 250.0);
        assert!(!path.with_extension("json.tmp").exists());
        let legacy = r#"{"version":1,"voice":{"boost":325},"profiles":{"mic-a":{"boost":225}},"start_audio":true}"#;
        std::fs::write(&path, legacy).unwrap();
        let migrated = Config::load_path(&path).unwrap();
        assert_eq!(migrated.version, CONFIG_VERSION);
        assert_eq!(migrated.voice.boost, 325.0);
        assert_eq!(migrated.saved_profiles["My setup"].boost, 325.0);
        assert_eq!(migrated.profiles["mic-a"].boost, 225.0);
        assert!(migrated.start_audio);
        assert!(migrated.default_profile.is_empty());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), legacy);
        for invalid in [
            "{broken".to_string(),
            format!("{{\"version\":{}}}", CONFIG_VERSION + 1),
        ] {
            std::fs::write(&path, &invalid).unwrap();
            assert!(Config::load_path(&path).is_err());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), invalid);
        }
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(&root).unwrap();
    }
}
