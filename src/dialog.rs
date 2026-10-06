//! File dialogs: rfd's on the desktop. The browser build has no file system to pick from, so
//! there it's a stand-in with the same builder API that never picks anything (and the UI hides
//! the buttons that would open one, see `AVAILABLE`).

#[cfg(target_arch = "wasm32")]
use std::path::PathBuf;

/// Whether file dialogs can open at all.
pub const AVAILABLE: bool = cfg!(not(target_arch = "wasm32"));

#[cfg(not(target_arch = "wasm32"))]
pub use rfd::FileDialog;

#[cfg(target_arch = "wasm32")]
pub struct FileDialog;

#[cfg(target_arch = "wasm32")]
impl FileDialog {
    pub fn new() -> Self {
        Self
    }

    pub fn add_filter(self, _name: impl Into<String>, _extensions: &[impl ToString]) -> Self {
        self
    }

    pub fn set_file_name(self, _name: impl Into<String>) -> Self {
        self
    }

    pub fn pick_file(self) -> Option<PathBuf> {
        None
    }

    pub fn pick_files(self) -> Option<Vec<PathBuf>> {
        None
    }

    pub fn pick_folder(self) -> Option<PathBuf> {
        None
    }

    pub fn save_file(self) -> Option<PathBuf> {
        None
    }
}
