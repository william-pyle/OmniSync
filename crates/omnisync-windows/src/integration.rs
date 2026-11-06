use anyhow::Result;
use std::path::Path;
use winreg::{enums::*, RegKey};
pub const OVERLAY_KEY: &str =
    r"SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\ShellIconOverlayIdentifiers";
pub const CLSIDS: [&str; 5] = [
    "{ADABEE52-131C-42ED-8310-9D35E3B77B01}",
    "{ADABEE52-131C-42ED-8310-9D35E3B77B02}",
    "{ADABEE52-131C-42ED-8310-9D35E3B77B03}",
    "{ADABEE52-131C-42ED-8310-9D35E3B77B04}",
    "{ADABEE52-131C-42ED-8310-9D35E3B77B05}",
];
pub const COMMAND_CLSIDS: [&str; 3] = [
    "{ADABEE52-131C-42ED-8310-9D35E3B77B11}",
    "{ADABEE52-131C-42ED-8310-9D35E3B77B12}",
    "{ADABEE52-131C-42ED-8310-9D35E3B77B13}",
];
pub const BADGES: [&str; 5] = ["Backed up", "Syncing", "Pending", "Excluded", "Issue"];
pub fn startup(enabled: bool, exe: &Path) -> Result<()> {
    let key = RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey(r"Software\Microsoft\Windows\CurrentVersion\Run")?
        .0;
    if enabled {
        key.set_value("OmniSync", &format!("\"{}\" --tray", exe.display()))?;
    } else {
        match key.delete_value("OmniSync") {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}
pub fn open_url(url: &str) -> Result<()> {
    use windows::{
        core::PCWSTR,
        Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
    };
    let allowed = url.starts_with("https://accounts.google.com/")
        || url.starts_with("https://drive.google.com/")
        || url.starts_with("https://one.google.com/");
    anyhow::ensure!(allowed, "Unsupported external URL");
    unsafe {
        let result = ShellExecuteW(
            None,
            windows::core::w!("open"),
            PCWSTR(crate::wide(url).as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
        anyhow::ensure!(result.0 as isize > 32, "Could not open the system browser");
    }
    Ok(())
}
