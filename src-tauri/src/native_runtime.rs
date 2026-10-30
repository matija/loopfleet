#[derive(serde::Serialize)]
pub struct Status {
    pub ready: bool,
    pub detail: String,
}

#[cfg(target_os = "macos")]
pub fn status() -> Status {
    #[repr(C, packed(2))]
    struct Descriptor {
        kind: u32,
        data: *mut std::ffi::c_void,
    }

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrusted() -> bool;
    }
    #[link(name = "CoreServices", kind = "framework")]
    extern "C" {
        fn AECreateDesc(kind: u32, data: *const u8, size: isize, result: *mut Descriptor) -> i16;
        fn AEDisposeDesc(desc: *mut Descriptor) -> i16;
        fn AEDeterminePermissionToAutomateTarget(
            target: *const Descriptor,
            event_class: u32,
            event_id: u32,
            ask: u8,
        ) -> i32;
    }

    let accessibility = unsafe { AXIsProcessTrusted() };
    let mut descriptor = Descriptor {
        kind: 0,
        data: std::ptr::null_mut(),
    };
    let target = b"com.apple.systemevents";
    let created = unsafe {
        AECreateDesc(
            u32::from_be_bytes(*b"bund"),
            target.as_ptr(),
            target.len() as isize,
            &mut descriptor,
        )
    };
    let automation = if created == 0 {
        let probe = || unsafe {
            AEDeterminePermissionToAutomateTarget(
                &descriptor,
                u32::from_be_bytes(*b"****"),
                u32::from_be_bytes(*b"****"),
                0,
            )
        };
        let mut result = probe();
        if result == -600 {
            let launched = std::process::Command::new("/usr/bin/open")
                .args(["-g", "-a", "/System/Library/CoreServices/System Events.app"])
                .status()
                .is_ok_and(|status| status.success());
            if launched {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
                while result == -600 && std::time::Instant::now() < deadline {
                    std::thread::sleep(std::time::Duration::from_millis(25));
                    result = probe();
                }
            }
        }
        unsafe {
            AEDisposeDesc(&mut descriptor);
        }
        result
    } else {
        i32::from(created)
    };
    let mut missing = Vec::new();
    if !accessibility {
        missing.push("Loopfleet needs Accessibility (Device Control and Data Access) in macOS System Settings > Privacy & Security".to_string());
    }
    if automation != 0 {
        missing.push(match automation {
            -600 => "System Events could not start; check native readiness again".to_string(),
            -1743 | -1744 => "Loopfleet needs Automation > System Events in macOS System Settings > Privacy & Security".to_string(),
            code => format!("Cannot verify System Events automation permission (macOS error {code})"),
        });
    }
    Status {
        ready: missing.is_empty(),
        detail: if missing.is_empty() {
            "Native UI automation is ready on this Mac.".into()
        } else {
            format!("Native UI setup required. {}. No agent run was started. Permissions belong to this Loopfleet installation, not individual PRDs. Check again after setup; replacing or re-signing Loopfleet may require renewed consent.", missing.join(". "))
        },
    }
}

#[cfg(not(target_os = "macos"))]
pub fn status() -> Status {
    Status {
        ready: false,
        detail: "Native UI automation is supported only on macOS.".into(),
    }
}
