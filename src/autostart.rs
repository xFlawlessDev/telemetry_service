use std::path::Path;

use tracing::warn;

use crate::error::{AppError, AppResult};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// SDDL access mask appended to the task security descriptor so that the
/// non-elevated activation agent can delete its own task after success.
/// FR = FILE_GENERIC_READ, FX = FILE_GENERIC_EXECUTE, SD = DELETE.
#[cfg(windows)]
const USER_DELETE_ACE: &str = "(A;;FRFXSD;;;AU)";

/// Well-known `BUILTIN\Users` SID; the task runs in the context of any user
/// that logs on, not in the context of the elevated account that installed it.
#[cfg(windows)]
const BUILTIN_USERS_SID: &str = "S-1-5-32-545";

/// Registry `Run` values written by the previous autostart implementation.
/// They are removed on install/remove so upgrades do not leave duplicates.
const LEGACY_RUN_HIVES: [&str; 2] = ["HKLM", "HKCU"];
const LEGACY_RUN_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Run";

pub fn install_autostart(entry_name: &str, executable: &Path) -> AppResult<()> {
    install_scheduled_task(entry_name, executable)?;
    remove_legacy_run_entries(entry_name);
    Ok(())
}

pub fn disable_autostart(entry_name: &str) -> AppResult<()> {
    let result = delete_scheduled_task(entry_name);
    remove_legacy_run_entries(entry_name);
    result
}

#[cfg(windows)]
fn install_scheduled_task(entry_name: &str, executable: &Path) -> AppResult<()> {
    use windows::Win32::System::TaskScheduler::{
        TASK_ACTION_EXEC, TASK_CREATE_OR_UPDATE, TASK_INSTANCES_IGNORE_NEW, TASK_LOGON_GROUP,
        TASK_RUNLEVEL_LUA, TASK_TRIGGER_LOGON,
    };
    use windows::core::{BSTR, Interface, VARIANT};

    let _com = ComGuard::init()?;
    let service = create_task_service()?;
    let empty = VARIANT::default();
    unsafe { service.Connect(&empty, &empty, &empty, &empty)? };

    let definition = unsafe { service.NewTask(0)? };

    unsafe {
        definition.RegistrationInfo()?.SetDescription(&BSTR::from(
            "Telemetry activation agent (runs once per user until activation succeeds).",
        ))?;

        let settings = definition.Settings()?;
        settings.SetEnabled(windows::Win32::Foundation::VARIANT_TRUE)?;
        settings.SetAllowDemandStart(windows::Win32::Foundation::VARIANT_TRUE)?;
        settings.SetStartWhenAvailable(windows::Win32::Foundation::VARIANT_TRUE)?;
        settings.SetDisallowStartIfOnBatteries(windows::Win32::Foundation::VARIANT_FALSE)?;
        settings.SetStopIfGoingOnBatteries(windows::Win32::Foundation::VARIANT_FALSE)?;
        settings.SetMultipleInstances(TASK_INSTANCES_IGNORE_NEW)?;
        // PT0S disables the time limit; the agent may retry until the server is reachable.
        settings.SetExecutionTimeLimit(&BSTR::from("PT0S"))?;

        let principal = definition.Principal()?;
        principal.SetGroupId(&BSTR::from(BUILTIN_USERS_SID))?;
        principal.SetLogonType(TASK_LOGON_GROUP)?;
        principal.SetRunLevel(TASK_RUNLEVEL_LUA)?;

        let triggers = definition.Triggers()?;
        let trigger = triggers.Create(TASK_TRIGGER_LOGON)?;
        trigger.SetEnabled(windows::Win32::Foundation::VARIANT_TRUE)?;
        trigger.SetStartBoundary(&BSTR::from("2020-01-01T00:00:00"))?;

        let actions = definition.Actions()?;
        let action = actions.Create(TASK_ACTION_EXEC)?;
        let exec = action.cast::<windows::Win32::System::TaskScheduler::IExecAction>()?;
        exec.SetPath(&BSTR::from(executable.display().to_string()))?;
        exec.SetArguments(&BSTR::from(""))?;
    }

    let folder = unsafe { service.GetFolder(&BSTR::from("\\"))? };
    let users = VARIANT::from(BSTR::from(BUILTIN_USERS_SID));
    let registered = unsafe {
        folder.RegisterTaskDefinition(
            &BSTR::from(entry_name),
            &definition,
            TASK_CREATE_OR_UPDATE.0,
            &users,
            &empty,
            TASK_LOGON_GROUP,
            &empty,
        )?
    };

    // Grant Authenticated Users the rights needed to delete the task from the
    // non-elevated user session, then persist the modified descriptor.
    let current = unsafe { registered.GetSecurityDescriptor(0xF)? };
    let sddl = append_user_delete_ace(&current.to_string());
    unsafe { registered.SetSecurityDescriptor(&BSTR::from(sddl), 0)? };

    Ok(())
}

#[cfg(windows)]
fn delete_scheduled_task(entry_name: &str) -> AppResult<()> {
    use windows::core::BSTR;

    let _com = ComGuard::init()?;
    let service = create_task_service()?;
    let empty = windows::core::VARIANT::default();
    unsafe { service.Connect(&empty, &empty, &empty, &empty)? };
    let folder = unsafe { service.GetFolder(&BSTR::from("\\"))? };

    match unsafe { folder.DeleteTask(&BSTR::from(entry_name), 0) } {
        Ok(()) => Ok(()),
        Err(error) if is_task_not_found(&error) => Ok(()),
        Err(error) => Err(AppError::Autostart(format!(
            "failed to delete scheduled task `{entry_name}`: {error}"
        ))),
    }
}

#[cfg(not(windows))]
fn install_scheduled_task(_entry_name: &str, _executable: &Path) -> AppResult<()> {
    Err(AppError::Autostart(
        "scheduled task autostart requires Windows".to_owned(),
    ))
}

#[cfg(not(windows))]
fn delete_scheduled_task(_entry_name: &str) -> AppResult<()> {
    Ok(())
}

fn remove_legacy_run_entries(entry_name: &str) {
    for hive in LEGACY_RUN_HIVES {
        let key = format!(r"{hive}\{LEGACY_RUN_KEY}");
        let mut command = std::process::Command::new("reg");
        command.args(["delete", &key, "/v", entry_name, "/f"]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(CREATE_NO_WINDOW);
        }

        match command.output() {
            Ok(output) if output.status.success() => {}
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                warn!(%error, hive, "legacy registry Run cleanup failed");
            }
        }
    }
}

#[cfg(windows)]
fn append_user_delete_ace(base: &str) -> String {
    format!("{base}{USER_DELETE_ACE}")
}

#[cfg(windows)]
fn is_task_not_found(error: &windows::core::Error) -> bool {
    error.code() == windows::core::HRESULT::from_win32(2)
}

#[cfg(windows)]
fn create_task_service() -> AppResult<windows::Win32::System::TaskScheduler::ITaskService> {
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
    use windows::Win32::System::TaskScheduler::{ITaskService, TaskScheduler};

    let service: ITaskService = unsafe {
        CoCreateInstance(
            &TaskScheduler,
            None::<&windows::core::IUnknown>,
            CLSCTX_INPROC_SERVER,
        )?
    };
    Ok(service)
}

#[cfg(windows)]
struct ComGuard {
    uninitialize: bool,
}

#[cfg(windows)]
impl ComGuard {
    fn init() -> AppResult<Self> {
        use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};

        let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if result.is_ok() {
            return Ok(Self { uninitialize: true });
        }
        // RPC_E_CHANGED_MODE: the thread is already initialized with a different
        // apartment model. Reuse it and skip CoUninitialize.
        if result == windows::core::HRESULT(0x8001_0106_u32 as i32) {
            return Ok(Self {
                uninitialize: false,
            });
        }
        Err(AppError::Autostart(format!(
            "failed to initialize COM: {result:?}"
        )))
    }
}

#[cfg(windows)]
impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.uninitialize {
            unsafe { windows::Win32::System::Com::CoUninitialize() };
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use super::*;

    #[cfg(windows)]
    #[test]
    fn append_user_delete_ace_should_grant_delete_to_authenticated_users() {
        let sddl = append_user_delete_ace("D:(A;;GA;;;SY)");

        assert_eq!(sddl, "D:(A;;GA;;;SY)(A;;FRFXSD;;;AU)");
        assert!(sddl.contains(";;;AU)"));
        assert!(sddl.contains("SD"));
    }
}
