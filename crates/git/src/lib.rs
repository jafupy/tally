//! Storage interface for Git; tally owns counting and diffing.
use gix_object::{FindExt, Kind};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

pub struct Repository {
    objects: gix_odb::HandleArc,
    alternate_objects: Vec<gix_odb::HandleArc>,
    ref_snapshot: Option<Arc<HashMap<String, gix_hash::ObjectId>>>,
    hash: gix_hash::Kind,
    gitdir: PathBuf,
    common: PathBuf,
    workdir: Option<PathBuf>,
    prefix: PathBuf,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct BlobId(gix_hash::ObjectId);
pub struct Blob(Vec<u8>);
impl Blob {
    pub fn content(&self) -> &[u8] {
        &self.0
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
        let explicit_gitdir = std::env::var_os("GIT_DIR")
            .map(PathBuf::from)
            .map(|path| path.canonicalize())
            .transpose()?;
        let explicit_workdir = std::env::var_os("GIT_WORK_TREE")
            .map(PathBuf::from)
            .map(|path| path.canonicalize())
            .transpose()?;
        let discovered = root
            .ancestors()
            .find(|p| p.join(".git").exists())
            .map(Path::to_path_buf)
            .or_else(|| {
                explicit_workdir.as_ref().and_then(|_| {
                    std::env::current_dir().ok().and_then(|cwd| {
                        cwd.ancestors()
                            .find(|p| p.join(".git").exists())
                            .map(Path::to_path_buf)
                    })
                })
            });
        let workdir = if let Some(workdir) = explicit_workdir {
            Some(workdir)
        } else if let Some(gitdir) = &explicit_gitdir {
            if config_value(&gitdir.join("config"), "core", "bare")?
                .is_some_and(|value| value.eq_ignore_ascii_case("true"))
            {
                None
            } else if let Some(path) = config_value(&gitdir.join("config"), "core", "worktree")? {
                Some(gitdir.join(path).canonicalize()?)
            } else {
                Some(std::env::current_dir()?.canonicalize()?)
            }
        } else {
            discovered.clone()
        };
        let gitdir = if let Some(gitdir) = explicit_gitdir {
            gitdir
        } else if let Some(discovered) = &discovered {
            let dotgit = discovered.join(".git");
            if dotgit.is_dir() {
                dotgit
            } else {
                let text = fs::read_to_string(&dotgit)?;
                let path = text.trim().strip_prefix("gitdir: ").ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "invalid .git file")
                })?;
                discovered.join(path).canonicalize()?
            }
        } else if root.join("HEAD").is_file() && root.join("objects").is_dir() {
            root.clone()
        } else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "not a Git repository",
            ));
        };
        let common = if let Some(path) = std::env::var_os("GIT_COMMON_DIR") {
            PathBuf::from(path).canonicalize()?
        } else {
            match fs::read_to_string(gitdir.join("commondir")) {
                Ok(path) => gitdir.join(path.trim()).canonicalize()?,
                Err(error) if error.kind() == io::ErrorKind::NotFound => gitdir.clone(),
                Err(error) => return Err(error),
            }
        };
        let hash = object_format(&common.join("config"))?;
        let ref_snapshot = reftable_refs(&gitdir, &common, hash)?;
        let replacements = replacement_refs(&gitdir, &common, hash, ref_snapshot.as_deref())?;
        let object_dir = std::env::var_os("GIT_OBJECT_DIRECTORY")
            .map(PathBuf::from)
            .unwrap_or_else(|| common.join("objects"));
        let objects = Arc::new(gix_odb::Store::at_opts(
            object_dir,
            hash,
            &mut replacements.clone().into_iter(),
            gix_odb::store::init::Options::default(),
        )?)
        .to_cache_arc();
        let mut alternate_objects = Vec::new();
        if let Some(paths) = std::env::var_os("GIT_ALTERNATE_OBJECT_DIRECTORIES") {
            for path in std::env::split_paths(&paths) {
                if !path.is_dir() {
                    continue;
                }
                alternate_objects.push(
                    Arc::new(gix_odb::Store::at_opts(
                        path,
                        hash,
                        &mut replacements.clone().into_iter(),
                        gix_odb::store::init::Options::default(),
                    )?)
                    .to_cache_arc(),
                );
            }
        }
        let prefix = workdir
            .as_ref()
            .map(|workdir| {
                root.strip_prefix(workdir)
                    .map(Path::to_path_buf)
                    .map_err(|_| {
                        io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "path is outside Git working tree",
                        )
                    })
            })
            .transpose()?
            .unwrap_or_default();
        Ok(Self {
            objects,
            alternate_objects,
            ref_snapshot,
            hash,
            gitdir,
            common,
            prefix,
            workdir,
        })
    }
    pub fn blob(&self, id: BlobId) -> io::Result<Blob> {
        let (kind, data) = self.object(id.0)?;
        if kind != Kind::Blob {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "expected Git blob",
            ));
        }
        Ok(Blob(data))
    }
    fn object(&self, id: gix_hash::ObjectId) -> io::Result<(Kind, Vec<u8>)> {
        let mut buf = Vec::new();
        match self.objects.find(&id, &mut buf) {
            Ok(object) => Ok((object.kind, object.data.to_vec())),
            Err(gix_object::find::existing::Error::NotFound { .. }) => {
                for alternate in &self.alternate_objects {
                    match alternate.find(&id, &mut buf) {
                        Ok(object) => return Ok((object.kind, object.data.to_vec())),
                        Err(gix_object::find::existing::Error::NotFound { .. }) => {}
                        Err(error) => return Err(git_error(error)),
                    }
                }
                self.git_object(id)
            }
            Err(error) => Err(git_error(error)),
        }
    }
    fn git_object(&self, id: gix_hash::ObjectId) -> io::Result<(Kind, Vec<u8>)> {
        let mut child = std::process::Command::new("git")
            .arg("--git-dir")
            .arg(&self.gitdir)
            .arg("cat-file")
            .arg("--batch")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;
        writeln!(child.stdin.take().unwrap(), "{id}")?;
        let output = child.wait_with_output()?;
        if !output.status.success() {
            return Err(io::Error::other(
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ));
        }
        let newline = output
            .stdout
            .iter()
            .position(|byte| *byte == b'\n')
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Git object is missing"))?;
        let header = std::str::from_utf8(&output.stdout[..newline]).map_err(git_error)?;
        let mut fields = header.split_whitespace();
        fields.next();
        let kind = match fields.next() {
            Some("blob") => Kind::Blob,
            Some("tree") => Kind::Tree,
            Some("commit") => Kind::Commit,
            Some("tag") => Kind::Tag,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "Git object is missing",
                ));
            }
        };
        let size = fields
            .next()
            .and_then(|size| size.parse::<usize>().ok())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid Git object size"))?;
        let body = &output.stdout[newline + 1..];
        if body.len() < size + 1 || body[size] != b'\n' {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "truncated Git object",
            ));
        }
        Ok((kind, body[..size].to_vec()))
    }
    pub fn files(
        &self,
        reference: &str,
        target: Option<&str>,
        include: impl Fn(&Path) -> bool,
    ) -> io::Result<Vec<FilePair>> {
        let mut files = BTreeMap::<PathBuf, FileVersions>::new();
        self.walk_tree(
            self.tree_for(reference)?,
            Path::new(""),
            &include,
            &mut |path, id| {
                files.entry(path).or_default().old = Some(id);
            },
        )?;
        if let Some(target) = target {
            self.walk_tree(
                self.tree_for(target)?,
                Path::new(""),
                &include,
                &mut |path, id| {
                    files.entry(path).or_default().new = Some(id);
                },
            )?;
        } else {
            if self.workdir.is_none() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "working-tree diff requires a working tree",
                ));
            }
            let index = self.index()?;
            for entry in index.entries() {
                if entry.mode.is_sparse() {
                    let path = path_from_bytes(entry.path(&index).as_ref());
                    self.walk_tree(entry.id, &path, &include, &mut |path, id| {
                        files.entry(path).or_default().new = Some(id);
                    })?;
                    continue;
                }
                if entry.mode.bits() & 0o170000 != 0o100000 {
                    continue;
                }
                let path = path_from_bytes(entry.path(&index).as_ref());
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
                working_tree: versions
                    .indexed
                    .then(|| self.workdir.as_ref().unwrap().join(path)),
            })
            .collect())
    }
    pub fn tracked_files(&self) -> io::Result<Vec<PathBuf>> {
        let index = self.index()?;
        let mut files = Vec::new();
        let mut seen = HashSet::new();
        for entry in index.entries() {
            let path = path_from_bytes(entry.path(&index).as_ref());
            if let Ok(relative) = path.strip_prefix(&self.prefix) {
                if seen.insert(relative.to_path_buf()) {
                    files.push(relative.to_path_buf());
                }
            }
        }
        Ok(files)
    }
    fn index(&self) -> io::Result<gix_index::File> {
        if self.workdir.is_none() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "tracked files require a working tree",
            ));
        }
        let path = std::env::var_os("GIT_INDEX_FILE")
            .map(PathBuf::from)
            .unwrap_or_else(|| self.gitdir.join("index"));
        gix_index::File::at(path, self.hash, true, gix_index::decode::Options::default())
            .map_err(git_error)
    }
    fn tree_for(&self, name: &str) -> io::Result<gix_hash::ObjectId> {
        let mut id = self.revision(name)?;
        for _ in 0..16 {
            let (kind, data) = self.object(id)?;
            match kind {
                Kind::Tree => return Ok(id),
                Kind::Commit => id = header_id(&data, b"tree ")?,
                Kind::Tag => id = header_id(&data, b"object ")?,
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "revision has no tree",
                    ));
                }
            }
        }
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "too many nested Git tags",
        ))
    }
    fn revision(&self, name: &str) -> io::Result<gix_hash::ObjectId> {
        if let Some(operator) = name.find(['~', '^'])
            && let Some(base) = self.named_revision(&name[..operator])?
            && let Some(id) = self.ancestry(base, &name[operator..])?
        {
            return Ok(id);
        }
        if let Some(id) = self.named_revision(name)? {
            return Ok(id);
        }
        // Keep Git's uncommon revision syntax through a single fallback call.
        let output = std::process::Command::new("git")
            .arg("--git-dir")
            .arg(&self.gitdir)
            .arg("rev-parse")
            .arg("--verify")
            .arg(format!("{name}^{{object}}"))
            .output()?;
        if !output.status.success() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ));
        }
        gix_hash::ObjectId::from_hex(output.stdout.trim_ascii()).map_err(git_error)
    }
    fn named_revision(&self, name: &str) -> io::Result<Option<gix_hash::ObjectId>> {
        if let Ok(id) = gix_hash::ObjectId::from_hex(name.as_bytes()) {
            return Ok(Some(id));
        }
        if std::env::var_os("GIT_NAMESPACE").is_some() {
            return Ok(None);
        }
        if !safe_ref_name(name) {
            return Ok(None);
        }
        let mut refs = Vec::new();
        if matches!(name, "HEAD" | "@") {
            refs.push("HEAD".to_owned());
        }
        if name.starts_with("refs/") {
            refs.push(name.to_owned());
        } else {
            refs.push(format!("refs/{name}"));
            refs.push(format!("refs/tags/{name}"));
            refs.push(format!("refs/heads/{name}"));
            refs.push(format!("refs/remotes/{name}"));
        }
        for reference in &refs {
            if let Some(id) = self.read_ref(reference, 0)? {
                return Ok(Some(id));
            }
        }
        Ok(None)
    }
    fn ancestry(
        &self,
        mut id: gix_hash::ObjectId,
        suffix: &str,
    ) -> io::Result<Option<gix_hash::ObjectId>> {
        let mut remaining = suffix.as_bytes();
        let mut operations = Vec::new();
        while !remaining.is_empty() {
            let operator = remaining[0];
            if operator != b'~' && operator != b'^' {
                return Ok(None);
            }
            remaining = &remaining[1..];
            let digits = remaining
                .iter()
                .take_while(|byte| byte.is_ascii_digit())
                .count();
            let count = if digits == 0 {
                1
            } else {
                let Ok(count) = std::str::from_utf8(&remaining[..digits])
                    .unwrap()
                    .parse::<usize>()
                else {
                    return Ok(None);
                };
                count
            };
            remaining = &remaining[digits..];
            if !remaining.is_empty() && remaining[0] != b'~' && remaining[0] != b'^' {
                return Ok(None);
            }
            operations.push((operator, count));
        }
        for (operator, count) in operations {
            id = self.peel_commit(id)?;
            if operator == b'~' {
                for _ in 0..count {
                    id = self.parent(id, 1)?;
                }
            } else if count != 0 {
                id = self.parent(id, count)?;
            }
        }
        Ok(Some(id))
    }
    fn peel_commit(&self, mut id: gix_hash::ObjectId) -> io::Result<gix_hash::ObjectId> {
        for _ in 0..16 {
            let (kind, data) = self.object(id)?;
            if kind == Kind::Commit {
                return Ok(id);
            }
            if kind != Kind::Tag {
                break;
            }
            id = header_id(&data, b"object ")?;
        }
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "revision is not a commit",
        ))
    }
    fn parent(&self, id: gix_hash::ObjectId, number: usize) -> io::Result<gix_hash::ObjectId> {
        let (_, data) = self.object(id)?;
        let line = data
            .split(|byte| *byte == b'\n')
            .filter(|line| line.starts_with(b"parent "))
            .nth(number - 1)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "commit has no requested parent",
                )
            })?;
        gix_hash::ObjectId::from_hex(&line[b"parent ".len()..]).map_err(git_error)
    }
    fn read_ref(&self, reference: &str, depth: usize) -> io::Result<Option<gix_hash::ObjectId>> {
        if depth > 16 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Git ref cycle"));
        }
        if reference == "HEAD"
            && let Some(id) = self.ref_snapshot.as_ref().and_then(|refs| refs.get("HEAD"))
        {
            return Ok(Some(*id));
        }
        for base in [&self.gitdir, &self.common] {
            match fs::read_to_string(base.join(reference)) {
                Ok(value) => {
                    let value = value.trim();
                    if let Some(next) = value.strip_prefix("ref: ") {
                        return self.read_ref(next, depth + 1);
                    }
                    return gix_hash::ObjectId::from_hex(value.as_bytes())
                        .map(Some)
                        .map_err(git_error);
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                    ) => {}
                Err(error) => return Err(error),
            }
        }
        if let Some(id) = self
            .ref_snapshot
            .as_ref()
            .and_then(|refs| refs.get(reference))
        {
            return Ok(Some(*id));
        }
        match fs::read_to_string(self.common.join("packed-refs")) {
            Ok(content) => {
                for line in content.lines() {
                    if line.starts_with('#') || line.starts_with('^') {
                        continue;
                    }
                    if let Some((id, name)) = line.split_once(' ') {
                        if name == reference {
                            return gix_hash::ObjectId::from_hex(id.as_bytes())
                                .map(Some)
                                .map_err(git_error);
                        }
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        Ok(None)
    }
    fn walk_tree(
        &self,
        id: gix_hash::ObjectId,
        parent: &Path,
        include: &impl Fn(&Path) -> bool,
        add: &mut impl FnMut(PathBuf, BlobId),
    ) -> io::Result<()> {
        let (kind, data) = self.object(id)?;
        if kind != Kind::Tree {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "expected Git tree",
            ));
        }
        for entry in gix_object::TreeRefIter::from_bytes(&data, self.hash) {
            let entry = entry.map_err(git_error)?;
            let path = parent.join(path_from_bytes(entry.filename.as_ref()));
            if entry.mode.is_tree() {
                self.walk_tree(entry.oid.to_owned(), &path, include, add)?;
            } else if entry.mode.is_blob() {
                if let Ok(relative) = path.strip_prefix(&self.prefix) {
                    if include(relative) {
                        add(path, BlobId(entry.oid.to_owned()));
                    }
                }
            }
        }
        Ok(())
    }
}
pub fn tracked_files(root: &Path) -> io::Result<Vec<PathBuf>> {
    Ok(Repository::open(root)?
        .tracked_files()?
        .into_iter()
        .map(|path| root.join(path))
        .collect())
}
fn header_id(data: &[u8], field: &[u8]) -> io::Result<gix_hash::ObjectId> {
    let line = data
        .split(|byte| *byte == b'\n')
        .find(|line| line.starts_with(field))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid Git object header"))?;
    gix_hash::ObjectId::from_hex(&line[field.len()..]).map_err(git_error)
}
fn git_error(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}
fn safe_ref_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains("@{")
        && !name.bytes().any(|byte| {
            byte <= b' '
                || matches!(
                    byte,
                    b':' | b'?' | b'*' | b'[' | b'\\' | b'^' | b'~' | b'\x7f'
                )
        })
        && name.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && !part.ends_with(".lock")
                && !part.ends_with('.')
        })
}
fn reftable_refs(
    gitdir: &Path,
    common: &Path,
    hash: gix_hash::Kind,
) -> io::Result<Option<Arc<HashMap<String, gix_hash::ObjectId>>>> {
    if !common.join("reftable").is_dir() && !gitdir.join("reftable").is_dir() {
        return Ok(None);
    }
    type RefMap = Arc<HashMap<String, gix_hash::ObjectId>>;
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, RefMap>>> = OnceLock::new();
    let mut cache = CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap();
    if let Some(refs) = cache.get(gitdir) {
        return Ok(Some(Arc::clone(refs)));
    }
    let output = std::process::Command::new("git")
        .arg("--git-dir")
        .arg(gitdir)
        .arg("show-ref")
        .arg("--head")
        .output()?;
    if !output.status.success()
        && !(output.status.code() == Some(1)
            && output.stdout.is_empty()
            && output.stderr.is_empty())
    {
        return Err(io::Error::other(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }
    let mut refs = HashMap::new();
    for line in output.stdout.split(|byte| *byte == b'\n') {
        let Some(space) = line.iter().position(|byte| *byte == b' ') else {
            continue;
        };
        let id = gix_hash::ObjectId::from_hex(&line[..space]).map_err(git_error)?;
        let name = std::str::from_utf8(line[space + 1..].trim_ascii()).map_err(git_error)?;
        if id.kind() == hash {
            refs.insert(name.to_owned(), id);
        }
    }
    let refs = Arc::new(refs);
    cache.insert(gitdir.to_path_buf(), Arc::clone(&refs));
    Ok(Some(refs))
}
fn replacement_refs(
    gitdir: &Path,
    common: &Path,
    hash: gix_hash::Kind,
    ref_snapshot: Option<&HashMap<String, gix_hash::ObjectId>>,
) -> io::Result<Vec<(gix_hash::ObjectId, gix_hash::ObjectId)>> {
    if std::env::var_os("GIT_NO_REPLACE_OBJECTS").is_some() {
        return Ok(Vec::new());
    }
    let base = std::env::var("GIT_REPLACE_REF_BASE").unwrap_or_else(|_| "refs/replace".to_owned());
    if !safe_ref_name(&base) {
        return Ok(Vec::new());
    }
    let prefix = format!("{base}/");
    let mut refs = BTreeMap::new();
    if let Some(snapshot) = ref_snapshot {
        for (name, target) in snapshot {
            if let Some(source) = name.strip_prefix(&prefix)
                && let Ok(source) = gix_hash::ObjectId::from_hex(source.as_bytes())
                && source.kind() == hash
                && target.kind() == hash
            {
                refs.insert(source, *target);
            }
        }
    } else {
        match fs::read_to_string(common.join("packed-refs")) {
            Ok(content) => {
                for line in content.lines() {
                    if let Some((target, name)) = line.split_once(' ')
                        && let Some(source) = name.strip_prefix(&prefix)
                        && let (Ok(source), Ok(target)) = (
                            gix_hash::ObjectId::from_hex(source.as_bytes()),
                            gix_hash::ObjectId::from_hex(target.as_bytes()),
                        )
                        && source.kind() == hash
                        && target.kind() == hash
                    {
                        refs.insert(source, target);
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        for dir in [common, gitdir] {
            match fs::read_dir(dir.join(&base)) {
                Ok(entries) => {
                    for entry in entries {
                        let entry = entry?;
                        let Some(source) = entry
                            .file_name()
                            .to_str()
                            .and_then(|name| gix_hash::ObjectId::from_hex(name.as_bytes()).ok())
                        else {
                            continue;
                        };
                        let target = fs::read_to_string(entry.path())?;
                        let target = gix_hash::ObjectId::from_hex(target.trim().as_bytes())
                            .map_err(git_error)?;
                        if source.kind() == hash && target.kind() == hash {
                            refs.insert(source, target);
                        }
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                    ) => {}
                Err(error) => return Err(error),
            }
        }
    }
    Ok(refs.into_iter().collect())
}
fn object_format(config: &Path) -> io::Result<gix_hash::Kind> {
    match config_value(config, "extensions", "objectformat")?
        .as_deref()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        None | Some("sha1") => Ok(gix_hash::Kind::Sha1),
        Some("sha256") => Ok(gix_hash::Kind::Sha256),
        Some(_) => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unsupported Git object format",
        )),
    }
}
fn config_value(config: &Path, section: &str, key: &str) -> io::Result<Option<String>> {
    let content = match fs::read_to_string(config) {
        Ok(content) => content,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut in_section = false;
    let mut found = None;
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_section = line
                .strip_prefix('[')
                .and_then(|line| line.split_once(']'))
                .is_some_and(|(name, _)| name.trim().eq_ignore_ascii_case(section));
        } else if in_section
            && let Some((entry_key, value)) = line.split_once('=')
            && entry_key.trim().eq_ignore_ascii_case(key)
        {
            found = Some(value.trim().trim_matches('"').to_owned());
        }
    }
    Ok(found)
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
