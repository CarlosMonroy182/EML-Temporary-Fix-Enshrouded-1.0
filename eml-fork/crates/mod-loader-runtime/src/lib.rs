use std::path::PathBuf;

use mod_loader::ModEnvironment;
// use parking_lot::Mutex;

mod log;
pub mod native;

pub struct RuntimeOptions {
    pub dlls: Vec<PathBuf>,
}

// static LIBRARIES: Mutex<Vec<libloading::Library>> = Mutex::new(Vec::new());

pub fn loader_attach(
    _env: &ModEnvironment,
    _options: RuntimeOptions,
) -> Result<(), Box<dyn std::error::Error>> {
    log::info!("Attaching runtime loader");

    native::attach();

    // let mut libraries = LIBRARIES.lock();
    //
    // for dll in options.dlls {
    //     log::info!(path = ?dll, "Loading DLL");
    //     let library = unsafe { libloading::Library::new(&dll) }?;
    //     libraries.push(library);
    // }

    Ok(())
}

pub fn loader_detach() -> Result<(), Box<dyn std::error::Error>> {
    log::info!("Detaching runtime loader");

    // LIBRARIES.lock().clear();

    Ok(())
}
