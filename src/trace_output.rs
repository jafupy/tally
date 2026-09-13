use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The trace file belongs to the working directory, even when scanning elsewhere.
pub struct TraceOutput {
    path: PathBuf,
    #[cfg(unix)]
    identity: Option<(u64, u64, u64)>,
}

impl TraceOutput {
    pub fn current() -> io::Result<Self> {
        let path = std::env::current_dir()?.canonicalize()?.join(".tallydebug");
        #[cfg(unix)]
        let identity = {
            use std::os::unix::fs::MetadataExt;
            fs::metadata(&path)
                .ok()
                .map(|metadata| (metadata.dev(), metadata.ino(), metadata.nlink()))
        };
        Ok(Self {
            path,
            #[cfg(unix)]
            identity,
        })
    }

    /// Directory entries are already under a canonical root. Only hard-linked
    /// aliases need a metadata lookup for names other than `.tallydebug`.
    pub fn matches_entry(&self, path: &Path) -> bool {
        if path.file_name() == Some(OsStr::new(".tallydebug")) {
            return self.matches_file(path);
        }
        #[cfg(unix)]
        if self.identity.is_some_and(|(_, _, links)| links > 1) {
            return self.same_inode(path);
        }
        false
    }

    /// Explicit input may use a relative path, symlink, or hard link.
    pub fn matches_file(&self, path: &Path) -> bool {
        path == self.path
            || path.file_name().is_some_and(|name| {
                path.parent()
                    .and_then(|parent| {
                        let parent = if parent.as_os_str().is_empty() {
                            Path::new(".")
                        } else {
                            parent
                        };
                        parent.canonicalize().ok()
                    })
                    .is_some_and(|parent| parent.join(name) == self.path)
            })
            || path
                .canonicalize()
                .is_ok_and(|resolved| resolved == self.path)
            || {
                #[cfg(unix)]
                {
                    self.same_inode(path)
                }
                #[cfg(not(unix))]
                {
                    false
                }
            }
    }

    #[cfg(unix)]
    fn same_inode(&self, path: &Path) -> bool {
        use std::os::unix::fs::MetadataExt;
        self.identity.is_some_and(|(dev, ino, _)| {
            fs::metadata(path).is_ok_and(|metadata| metadata.dev() == dev && metadata.ino() == ino)
        })
    }
}
