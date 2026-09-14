//! Git revision, tracked-file, and blob access for tally.

use git2::{Oid, Tree, TreeWalkMode, TreeWalkResult};
use std::{
    collections::{BTreeMap, HashSet},
    io,
    path::{Path, PathBuf},
};

pub struct Repository {
    inner: git2::Repository,
    workdir: PathBuf,
    prefix: PathBuf,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct BlobId(Oid);

pub struct Blob<'repo>(git2::Blob<'repo>);

impl Blob<'_> {
    pub fn content(&self) -> &[u8] {
        self.0.content()
    }
}

pub struct FilePair {
    pub path: PathBuf,
    pub old: Option<BlobId>,
    pub new: Option<BlobId>,
    pub working_tree: Option<PathBuf>,
}

#[derive(Default)]
struct FileVersions {
    old: Option<BlobId>,
    new: Option<BlobId>,
    indexed: bool,
}

impl Repository {
    pub fn open(root: &Path) -> io::Result<Self> {
        let root = root.canonicalize()?;
        let inner = git2::Repository::discover(&root).map_err(io::Error::other)?;
        let workdir = inner
            .workdir()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "not a Git working tree"))?
            .canonicalize()?;
        let prefix = root
            .strip_prefix(&workdir)
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "path is outside Git working tree",
                )
            })?
            .to_path_buf();
        Ok(Self {
            inner,
            workdir,
            prefix,
        })
    }

    pub fn blob(&self, id: BlobId) -> io::Result<Blob<'_>> {
        self.inner
            .find_blob(id.0)
            .map(Blob)
            .map_err(io::Error::other)
    }

    pub fn files(
        &self,
        reference: &str,
        target: Option<&str>,
        include: impl Fn(&Path) -> bool,
    ) -> io::Result<Vec<FilePair>> {
        let old_tree = self
            .inner
            .revparse_single(reference)
            .and_then(|object| object.peel_to_tree())
            .map_err(io::Error::other)?;
        let new_tree = target
            .map(|name| {
                self.inner
                    .revparse_single(name)
                    .and_then(|object| object.peel_to_tree())
                    .map_err(io::Error::other)
            })
            .transpose()?;
        let mut files = BTreeMap::<PathBuf, FileVersions>::new();
        self.walk_tree(&old_tree, &include, |path, id| {
            files.entry(path).or_default().old = Some(id);
        })?;
        if let Some(tree) = new_tree.as_ref() {
            self.walk_tree(tree, &include, |path, id| {
                files.entry(path).or_default().new = Some(id);
            })?;
        } else {
            let index = self.inner.index().map_err(io::Error::other)?;
            for entry in index.iter() {
                if entry.mode & 0o170000 != 0o100000 {
                    continue;
                }
                let path = path_from_bytes(&entry.path);
                if let Ok(relative) = path.strip_prefix(&self.prefix) {
                    if include(relative) {
                        files.entry(path).or_default().indexed = true;
                    }
                }
            }
        }
        Ok(files
            .into_iter()
            .map(|(path, versions)| FilePair {
                path: path.strip_prefix(&self.prefix).unwrap().to_path_buf(),
                old: versions.old,
                new: versions.new,
                working_tree: versions.indexed.then(|| self.workdir.join(path)),
            })
            .collect())
    }

    pub fn tracked_files(&self) -> io::Result<Vec<PathBuf>> {
        let index = self.inner.index().map_err(io::Error::other)?;
        let mut files = Vec::new();
        let mut seen = HashSet::new();
        for entry in index.iter() {
            let path = path_from_bytes(&entry.path);
            let Ok(relative) = path.strip_prefix(&self.prefix) else {
                continue;
            };
            if seen.insert(relative.to_path_buf()) {
                files.push(relative.to_path_buf());
            }
        }
        Ok(files)
    }

    fn walk_tree(
        &self,
        tree: &Tree<'_>,
        include: &impl Fn(&Path) -> bool,
        mut add: impl FnMut(PathBuf, BlobId),
    ) -> io::Result<()> {
        tree.walk(TreeWalkMode::PreOrder, |parent, entry| {
            if entry.filemode() & 0o170000 != 0o100000 {
                return TreeWalkResult::Ok;
            }
            let mut bytes = parent.as_bytes().to_vec();
            bytes.extend_from_slice(entry.name_bytes());
            let path = path_from_bytes(&bytes);
            if let Ok(relative) = path.strip_prefix(&self.prefix) {
                if include(relative) {
                    add(path, BlobId(entry.id()));
                }
            }
            TreeWalkResult::Ok
        })
        .map_err(io::Error::other)
    }
}

pub fn tracked_files(root: &Path) -> io::Result<Vec<PathBuf>> {
    Ok(Repository::open(root)?
        .tracked_files()?
        .into_iter()
        .map(|path| root.join(path))
        .collect())
}

#[cfg(unix)]
fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    PathBuf::from(std::ffi::OsString::from_vec(bytes.to_vec()))
}

#[cfg(not(unix))]
fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
}
