use anyhow::{Context, Result};
mod verify;
use omnisync_windows::registration::{self, Registration};
use std::{io::Write, path::Path};
fn main() {
    if let Err(error) = run() {
        eprintln!("OmniSync setup: {error:#}");
        std::process::exit(1);
    }
}
fn arg(args: &[String], index: usize) -> Result<&str> {
    args.get(index)
        .map(String::as_str)
        .context("Missing setup argument")
}
fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str){
    Some("inventory")=>{let items=registration::inventory()?;if args.len()==1{println!("{}",serde_json::to_string_pretty(&items)?);}else{let directory=Path::new(arg(&args,1)?);std::fs::create_dir_all(directory)?;std::fs::write(directory.join("inventory.json"),serde_json::to_vec_pretty(&items)?)?;let text=items.iter().enumerate().map(|(i,r)|format!("{}: {}{}\r\n  CLSID: {}\r\n  DLL: {}\r\n",i+1,r.name,if r.eligible{""}else{" [protected]"},r.clsid,r.dll)).collect::<Vec<_>>().join("\r\n");let mut file=std::fs::File::create(directory.join("inventory.txt"))?;file.write_all(&[0xff,0xfe])?;for word in text.encode_utf16(){file.write_all(&word.to_le_bytes())?;}}},
    Some("takeover")=>{let inventory:Vec<Registration>=serde_json::from_slice(&std::fs::read(arg(&args,1)?)?)?;let numbers=arg(&args,2)?;let mut selected=vec![];for token in numbers.split(',').map(str::trim).filter(|s|!s.is_empty()){let index=token.parse::<usize>().context("Enter registration numbers separated by commas")?.checked_sub(1).context("Registration numbers start at 1")?;let entry=inventory.get(index).context("Unknown registration number")?;anyhow::ensure!(entry.eligible,"A selected registration is protected");anyhow::ensure!(!selected.iter().any(|(name,_)|name==&entry.name),"Duplicate selection");selected.push((entry.name.clone(),entry.digest.clone()));}let path=registration::take_over(&selected)?;println!("{}",path.display());},
    Some("takeover-json")=>{let selected:Vec<(String,String)>=serde_json::from_slice(&std::fs::read(arg(&args,1)?)?)?;println!("{}",registration::take_over(&selected)?.display());},
    Some("restore")=>{let skipped=registration::restore_receipt(Path::new(arg(&args,1)?))?;if !skipped.is_empty(){anyhow::bail!("Newer/existing provider keys were preserved: {}",skipped.join(", "));}println!("Overlay registrations restored. Sign out or refresh Explorer when convenient.");},
    Some("register")=>{let directory=std::env::current_exe()?.parent().context("Setup location missing")?.to_path_buf();registration::register(&directory,args.iter().any(|s|s=="--overlays"),args.iter().any(|s|s=="--context"))?;},
    Some("unregister")=>{let directory=std::env::current_exe()?.parent().context("Setup location missing")?.to_path_buf();registration::unregister(&directory)?;},
    Some("probe-running")=>{use windows::{core::PCWSTR,Win32::{Foundation::CloseHandle,System::Threading::{OpenMutexW,SYNCHRONIZATION_ACCESS_RIGHTS}}};let name=omnisync_windows::wide("Global\\OmniSync-App-Running");match unsafe{OpenMutexW(SYNCHRONIZATION_ACCESS_RIGHTS(0x00100000),false,PCWSTR(name.as_ptr()))}{Ok(handle)=>{unsafe{let _=CloseHandle(handle);}std::process::exit(2);},Err(error)if error.code()==windows::Win32::Foundation::E_ACCESSDENIED=>std::process::exit(2),Err(_)=>{}}},
    Some("verify-shell")=>{verify::shell()?;println!("Verified 5 overlay classes, 3 context commands, icon resources, unmanaged-path suppression and COM lifetime. No registry writes.");},
    Some("--version")=>println!("OmniSync setup {}",env!("CARGO_PKG_VERSION")),
    _=>anyhow::bail!("Usage: omnisync-setup inventory [directory] | takeover inventory.json numbers | takeover-json selections.json | restore receipt.json | register [--overlays] [--context] | unregister | probe-running")
}
    Ok(())
}
