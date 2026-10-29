use std::path::PathBuf;

use mod_loader::ModEnvironment;
// use parking_lot::Mutex;

mod log;
pub mod native;

pub struct RuntimeOptions {
    pub dlls: Vec<PathBuf>,
}

// static LIBRARIES: Mutex<Vec<libloading::Library>> = Mutex::new(Vec::new());

/// Loads a native library that a mod registered with `loader.runtime.register_dll`.
/// A library that fails to load only gets logged; it never stops the game.
fn load_registered(path: &std::path::Path) {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        unsafe extern "system" {
            fn LoadLibraryW(name: *const u16) -> *mut core::ffi::c_void;
            fn GetLastError() -> u32;
        }
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let handle = std::panic::catch_unwind(|| unsafe { LoadLibraryW(wide.as_ptr()) });
        match handle {
            Ok(h) if !h.is_null() => {
                log::info!("Loaded native library {}", path.display());
            }
            Ok(_) => log::error!("Could not load native library {} (error {})", path.display(), unsafe { GetLastError() }),
            Err(_) => log::error!("Loading native library {} failed", path.display()),
        }
    }
    #[cfg(not(windows))]
    log::info!("Native library {} is only loaded on Windows", path.display());
}

pub fn loader_attach(
    _env: &ModEnvironment,
    _options: RuntimeOptions,
) -> Result<(), Box<dyn std::error::Error>> {
    log::info!("Attaching runtime loader");

    native::attach();

    for dll in _options.dlls {
        load_registered(&dll);
    }

    Ok(())
}

pub fn loader_detach() -> Result<(), Box<dyn std::error::Error>> {
    log::info!("Detaching runtime loader");

    // LIBRARIES.lock().clear();

    Ok(())
}
