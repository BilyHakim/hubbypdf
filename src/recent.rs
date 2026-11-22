use std::{
    ffi::OsString,
    fs::{self, OpenOptions},
    io,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
};

pub const LIMIT: usize = 10;

#[derive(Default)]
pub struct RecentPdfs {
    file: Option<PathBuf>,
    paths: Vec<PathBuf>,
}

impl RecentPdfs {
    pub fn load(file: Option<PathBuf>) -> Self {
        let paths = file.as_deref().map(read_paths).unwrap_or_default();
        Self { file, paths }
    }

    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    pub fn remember(&mut self, path: &Path) -> io::Result<()> {
        let path = std::path::absolute(path)?;
        self.paths.retain(|entry| entry != &path);
        self.paths.insert(0, path.clone());
        self.paths.truncate(LIMIT);
        if let Some(file) = &self.file {
            self.paths = write_paths(file, path, &self.paths)?;
        }
        Ok(())
    }
}

fn read_paths(file: &Path) -> Vec<PathBuf> {
    // Keep Windows paths lossless, including paths that cannot be represented as UTF-8.
    let bytes = fs::read(file).unwrap_or_default();
    let units: Vec<_> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    let mut paths = Vec::new();
    for entry in units.split_inclusive(|unit| *unit == 0) {
        let Some(entry) = entry.strip_suffix(&[0]) else {
            break;
        };
        let path = PathBuf::from(OsString::from_wide(entry));
        if path.is_absolute() && !paths.contains(&path) {
            paths.push(path);
            if paths.len() == LIMIT {
                break;
            }
        }
    }
    paths
}

fn write_paths(file: &Path, latest: PathBuf, session: &[PathBuf]) -> io::Result<Vec<PathBuf>> {
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir)?;
    }
    // Lock a separate file so simultaneous instances can merge and replace the history.
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(file.with_extension("lock"))?;
    lock.lock()?;
    let mut paths = vec![latest];
    for path in read_paths(file).into_iter().chain(session.iter().cloned()) {
        if !paths.contains(&path) {
            paths.push(path);
        }
        if paths.len() == LIMIT {
            break;
        }
    }
    let bytes: Vec<_> = paths
        .iter()
        .flat_map(|path| path.as_os_str().encode_wide().chain([0]))
        .flat_map(u16::to_le_bytes)
        .collect();
    let temp = file.with_extension("tmp");
    fs::write(&temp, bytes)?;
    fs::rename(temp, file)?;
    Ok(paths)
}
