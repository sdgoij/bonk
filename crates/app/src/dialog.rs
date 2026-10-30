//! Native file dialogs.
//!
//! `rfd` shows the platform's own picker: the Windows common dialog, the macOS
//! open and save panels, and the XDG portal on Linux. Nothing here holds a path
//! between frames; a cancelled dialog returns `None` and the caller does
//! nothing, which is why the window no longer needs a path field for a file the
//! user only chooses once.

use std::path::PathBuf;

/// Picks an existing file, filtered to `extensions`.
pub(crate) fn open(description: &str, extensions: &[&str]) -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title(description)
        .add_filter(description, extensions)
        .pick_file()
}

/// Picks where a file of `extension` should be written, suggesting `file_name`.
pub(crate) fn save(description: &str, file_name: &str, extension: &str) -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title(description)
        .set_file_name(file_name)
        .add_filter(description, &[extension])
        .save_file()
}
