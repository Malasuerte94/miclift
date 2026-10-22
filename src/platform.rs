use anyhow::{Context, Result, bail};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
};
use windows::{
    Win32::{
        Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, WAIT_OBJECT_0},
        System::Threading::{CreateEventW, CreateMutexW, SetEvent, WaitForSingleObject},
        UI::{
            Shell::ShellExecuteW,
            WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW, SW_SHOWNORMAL},
        },
    },
    core::{HSTRING, w},
};

pub fn error_dialog(message: &str) {
    unsafe {
        MessageBoxW(
            None,
            &HSTRING::from(message),
            w!("MicLift"),
            MB_OK | MB_ICONERROR,
        );
    }
}
pub fn open_url(url: &str) -> Result<()> {
    anyhow::ensure!(
        url.starts_with("https://"),
        "Only HTTPS links are supported"
    );
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            &HSTRING::from(url),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };
    anyhow::ensure!(result.0 as isize > 32, "Unable to open the default browser");
    Ok(())
}

pub struct Instance {
    mutex: HANDLE,
    event: HANDLE,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl Instance {
    pub fn acquire() -> Result<Option<Self>> {
        let mutex = unsafe { CreateMutexW(None, false, w!("Local\\MicLift.Instance.v1"))? };
        let exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        let event = match unsafe { CreateEventW(None, false, false, w!("Local\\MicLift.Show.v1")) }
        {
            Ok(event) => event,
            Err(error) => {
                unsafe {
                    let _ = CloseHandle(mutex);
                }
                return Err(error.into());
            }
        };
        if exists {
            unsafe {
                let _ = SetEvent(event);
                let _ = CloseHandle(event);
                let _ = CloseHandle(mutex);
            }
            return Ok(None);
        }
        Ok(Some(Self {
            mutex,
            event,
            stop: Arc::new(AtomicBool::new(false)),
            worker: None,
        }))
    }
    pub fn on_show(&mut self, callback: impl Fn() + Send + 'static) {
        let event = self.event.0 as usize;
        let stop = self.stop.clone();
        self.worker = Some(thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                if unsafe { WaitForSingleObject(HANDLE(event as *mut _), 250) } == WAIT_OBJECT_0
                    && !stop.load(Ordering::Relaxed)
                {
                    callback();
                }
            }
        }));
    }
}
impl Drop for Instance {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        unsafe {
            let _ = SetEvent(self.event);
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        unsafe {
            let _ = CloseHandle(self.event);
            let _ = CloseHandle(self.mutex);
        }
    }
}

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
pub fn autostart_enabled() -> bool {
    winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .open_subkey(RUN_KEY)
        .ok()
        .and_then(|key| key.get_value::<String, _>("MicLift").ok())
        .is_some()
}
pub fn set_autostart(enabled: bool) -> Result<()> {
    let (key, _) =
        winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER).create_subkey(RUN_KEY)?;
    if enabled {
        let executable = std::env::current_exe()?;
        key.set_value(
            "MicLift",
            &format!("\"{}\" --autostart", executable.display()),
        )?;
    } else {
        match key.delete_value("MicLift") {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

pub fn driver_setup() -> Result<String> {
    if !cfg!(feature = "licensed-driver-setup") {
        open_url("https://vb-audio.com/Cable/")?;
        return Ok("Official VB-CABLE page opened. Install the driver as administrator, restart if requested, then return to MicLift. Automatic integration is disabled in this build.".into());
    }
    use std::os::windows::process::CommandExt;
    let temporary: PathBuf =
        std::env::temp_dir().join(format!("miclift-setup-{}", std::process::id()));
    std::fs::create_dir_all(&temporary)?;
    let script = temporary.join("install-vbcable.ps1");
    std::fs::write(&script, include_str!("../tools/install-vbcable.ps1"))?;
    let result = std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&script)
        .arg("-WorkDirectory")
        .arg(&temporary)
        .creation_flags(0x08000000)
        .output()
        .context("Unable to start driver setup")?;
    let _ = std::fs::remove_file(script);
    if !result.status.success() {
        bail!(
            "VB-CABLE setup failed or was cancelled: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let message = String::from_utf8_lossy(&result.stdout).trim().to_string();
    if crate::audio::devices().is_ok_and(|devices| {
        devices.outputs.iter().any(|device| device.is_cable)
            && devices.inputs.iter().any(|device| device.is_cable)
    }) {
        Ok(format!(
            "{message}\nVB-CABLE endpoints are now visible to MicLift."
        ))
    } else {
        Ok(format!(
            "{message}\nThe driver endpoints are not visible yet. Restart Windows if installation completed, or retry setup if it was cancelled."
        ))
    }
}

pub fn process_resources() -> Result<(u64, f64)> {
    use windows::Win32::{
        Foundation::FILETIME,
        System::{
            ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX},
            Threading::{GetCurrentProcess, GetProcessTimes},
        },
    };
    let process = unsafe { GetCurrentProcess() };
    let mut memory = PROCESS_MEMORY_COUNTERS_EX {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        ..Default::default()
    };
    let (mut created, mut exited, mut kernel, mut user) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    unsafe {
        GetProcessMemoryInfo(
            process,
            (&mut memory as *mut PROCESS_MEMORY_COUNTERS_EX).cast(),
            memory.cb,
        )?;
        GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user)?;
    }
    let seconds = |time: FILETIME| {
        (((time.dwHighDateTime as u64) << 32) | time.dwLowDateTime as u64) as f64 / 10_000_000.0
    };
    Ok((memory.PrivateUsage as u64, seconds(kernel) + seconds(user)))
}
