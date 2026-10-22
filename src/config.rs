use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub const CONFIG_VERSION: u32 = 1;

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
        let boost = self.boost;
        *self = Self::default();
        self.boost = boost;
        match name {
            "Voice" => {
                self.high_pass = true;
                for (band, gain) in self.bands.iter_mut().zip([-3.0, -2.0, 1.0, 3.0, 1.0]) {
                    band.gain_db = gain;
                }
            }
            "Warm" => {
                for (band, gain) in self.bands.iter_mut().zip([3.0, 1.5, 0.0, -1.0, -2.0]) {
                    band.gain_db = gain;
                }
            }
            _ => {}
        }
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
        if config.version != CONFIG_VERSION {
            bail!(
                "Unsupported settings version {}; settings preserved",
                config.version
            );
        }
        config.voice.sanitize();
        config.monitor_volume = bounded(config.monitor_volume, 0.0, 1.0, 0.5);
        for voice in config.profiles.values_mut() {
            voice.sanitize();
        }
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
}

#[cfg(test)]
mod tests {
    use super::*;
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
        config.save_path(&path).unwrap();
        assert_eq!(Config::load_path(&path).unwrap().voice.boost, 425.0);
        config.voice.boost = 250.0;
        config.save_path(&path).unwrap();
        assert_eq!(Config::load_path(&path).unwrap().voice.boost, 250.0);
        assert!(!path.with_extension("json.tmp").exists());
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
