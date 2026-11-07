//! Layout templates stored beside the archive.
//!
//! A template is data, not code. The caller supplies the database path; the
//! directory is `<archive>/templates`, and a document names a layout by the file
//! stem it was saved under.

use std::path::{Path, PathBuf};

/// A layout read from the archive, with the file it came from and the digest
/// that identifies it.
#[derive(Clone, Debug)]
pub struct ArchiveTemplate {
    /// The file's stem, which is how a document names the layout it wants.
    pub name: String,
    /// The layout it describes.
    pub template: render::Template,
    /// Hex SHA-256 of the file, so a document can say which layout drew it.
    pub sha256: String,
}

impl ArchiveTemplate {
    /// The digest shortened for a label; the full one belongs in a record.
    pub fn short_sha256(&self) -> &str {
        self.sha256.get(..12).unwrap_or(self.sha256.as_str())
    }

    /// How the layout is named in a list: the file names it, and the template's
    /// own name is what the reader recognises. They are often the same word.
    pub fn label(&self) -> String {
        if self.template.name == self.name {
            self.template.name.clone()
        } else {
            format!("{} ({})", self.template.name, self.name)
        }
    }
}

/// The file name a document means when it asks for no particular layout.
pub const DEFAULT_TEMPLATE: &str = "default";

/// Every layout in `<archive>/templates`, sorted by file name.
///
/// One unreadable file does not hide the rest: a broken template is reported and
/// skipped, because an invoice drawn by a layout the user did not choose is
/// worse than a message saying one file is broken. Editing a file changes its
/// digest, which is why the digest is taken from the bytes on disk.
pub fn load_templates(database_path: &Path) -> (Vec<ArchiveTemplate>, Vec<String>) {
    let directory = database_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("templates");

    let entries = match std::fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return (Vec::new(), Vec::new()),
        Err(error) => return (Vec::new(), vec![format!("{}: {error}", directory.display())]),
    };

    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "json"))
        .collect();
    paths.sort();

    let mut templates = Vec::new();
    let mut problems = Vec::new();
    for path in paths {
        let Some(name) = path.file_stem().and_then(|stem| stem.to_str()).map(str::to_owned) else {
            problems.push(format!("{}: the file name is not text", path.display()));
            continue;
        };
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                problems.push(format!("{}: {error}", path.display()));
                continue;
            }
        };
        match std::str::from_utf8(&bytes) {
            Ok(json) => match render::Template::parse(json) {
                Ok(template) => templates.push(ArchiveTemplate {
                    name,
                    template,
                    sha256: sha256_hex(&bytes),
                }),
                Err(error) => problems.push(format!("{}: {error}", path.display())),
            },
            Err(error) => problems.push(format!("{}: {error}", path.display())),
        }
    }
    (templates, problems)
}

/// The layout an archive draws with when a document names none.
pub fn default_template(templates: &[ArchiveTemplate]) -> Option<&ArchiveTemplate> {
    templates.iter().find(|template| template.name == DEFAULT_TEMPLATE)
}

/// The renderer a template calls for: the archive's layout, or the built-in one.
pub fn renderer_for(template: Option<&render::Template>) -> render::Renderer {
    template.map_or_else(render::Renderer::new, |template| {
        render::Renderer::with_template(template.clone())
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}
