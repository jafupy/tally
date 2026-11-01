use super::{Blob, BlobId, FilePair, Repository, git_error};
use gix_object::{Find, Kind};
use std::{
    fs,
    io::{self, Write},
    process::{Command, Stdio},
    sync::Arc,
    thread,
};

impl Repository {
    pub fn worktree_blob(&self, id: BlobId) -> io::Result<Blob> {
        let mut buffer = Vec::new();
        for (_, objects) in self.worktree_objects.iter().rev() {
            if let Some(object) = objects.try_find(&id.0, &mut buffer).map_err(git_error)? {
                if object.kind != Kind::Blob {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "expected Git blob",
                    ));
                }
                return Ok(Blob(object.data.to_vec()));
            }
        }
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "working-tree blob is missing",
        ))
    }

    pub(super) fn clean_worktree_files(&mut self, files: &mut [FilePair]) -> io::Result<()> {
        let Some(root) = self.workdir.as_ref() else {
            return Ok(());
        };
        let mut selected = Vec::new();
        let mut input = Vec::new();
        for (i, file) in files.iter().enumerate() {
            let Some(path) = &file.working_tree else {
                continue;
            };
            if !fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_file()) {
                continue;
            }
            let relative = path.strip_prefix(root).map_err(git_error)?;
            input.extend_from_slice(relative.as_os_str().as_encoded_bytes());
            input.push(0);
            selected.push(i);
        }
        if selected.is_empty() {
            return Ok(());
        }

        // Let Git apply its exact clean rules without writing to the repository or index.
        let objects = Arc::new(tempfile::tempdir()?);
        let index_path = objects.path().join("index");
        let mut index = self.index()?;
        // Force reading selected files, even when their cached stat or flags say unchanged.
        for entry in index.entries_mut() {
            if entry.mode.bits() & 0o170000 == 0o100000 {
                entry.stat = Default::default();
                entry.flags.remove(
                    gix_index::entry::Flags::ASSUME_VALID
                        | gix_index::entry::Flags::SKIP_WORKTREE
                        | gix_index::entry::Flags::FSMONITOR_VALID,
                );
            }
        }
        index
            .write_to(
                fs::File::create(&index_path)?,
                gix_index::write::Options {
                    extensions: gix_index::write::Extensions::None,
                    ..Default::default()
                },
            )
            .map_err(git_error)?;
        fs::create_dir(objects.path().join("info"))?;
        let mut alternates = Vec::new();
        for odb in std::iter::once(&self.objects).chain(&self.alternate_objects) {
            let path = odb.store_ref().path().canonicalize()?;
            alternates.extend_from_slice(&gix_quote::ansi_c::quote(
                path.as_os_str().as_encoded_bytes().into(),
            ));
            alternates.push(b'\n');
        }
        fs::write(objects.path().join("info/alternates"), alternates)?;
        let mut child = Command::new("git")
            .current_dir(root)
            .arg("--git-dir")
            .arg(&self.gitdir)
            .arg("--work-tree")
            .arg(root)
            .args([
                "-c",
                "core.safecrlf=false",
                "-c",
                "core.splitIndex=false",
                "-c",
                "core.fsmonitor=false",
                "update-index",
                "--add",
                "--replace",
                "-z",
                "--stdin",
            ])
            .env("GIT_COMMON_DIR", &self.common)
            .env("GIT_INDEX_FILE", &index_path)
            .env("GIT_OBJECT_DIRECTORY", objects.path())
            .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let mut stdin = child.stdin.take().unwrap();
        // Drain stdout while feeding paths so large trees cannot fill both pipes.
        let writer = thread::spawn(move || stdin.write_all(&input));
        let output = child.wait_with_output();
        let written = writer.join().unwrap();
        let output = output?;
        if !output.status.success() {
            return Err(io::Error::other(
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ));
        }
        written?;
        let index = gix_index::File::at(
            index_path,
            self.hash,
            true,
            gix_index::decode::Options::default(),
        )
        .map_err(git_error)?;
        let store = Arc::new(gix_odb::Store::at_opts(
            objects.path().to_path_buf(),
            self.hash,
            &mut std::iter::empty(),
            gix_odb::store::init::Options::default(),
        )?)
        .to_cache_arc();
        self.worktree_objects.push((objects, store));
        for i in selected {
            let path = files[i].working_tree.as_ref().unwrap();
            let relative = path.strip_prefix(root).map_err(git_error)?;
            let entry = index
                .entry_by_path(relative.as_os_str().as_encoded_bytes().into())
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::NotFound, "cleaned Git blob is missing")
                })?;
            files[i].new = Some(BlobId(entry.id));
        }
        Ok(())
    }
}
