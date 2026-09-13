use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub(super) struct Rules {
    parent: Option<Arc<Rules>>,
    // Git rules stop here; .ignore rules still consult the parent chain.
    git_root: bool,
    ignore: Gitignore,
    gitignore: Gitignore,
    exclude: Gitignore,
}

impl Rules {
    pub(super) fn ignored(&self, path: &Path, is_dir: bool, global: &Gitignore) -> bool {
        let mut rules = Some(self);
        while let Some(current) = rules {
            let matched = current.ignore.matched(path, is_dir);
            if !matched.is_none() {
                return matched.is_ignore();
            }
            rules = current.parent.as_deref();
        }
        rules = Some(self);
        while let Some(current) = rules {
            let matched = current.gitignore.matched(path, is_dir);
            if !matched.is_none() {
                return matched.is_ignore();
            }
            if current.git_root {
                break;
            }
            rules = current.parent.as_deref();
        }
        rules = Some(self);
        while let Some(current) = rules {
            let matched = current.exclude.matched(path, is_dir);
            if !matched.is_none() {
                return matched.is_ignore();
            }
            if current.git_root {
                break;
            }
            rules = current.parent.as_deref();
        }
        global.matched(path, is_dir).is_ignore()
    }
}

fn matcher(path: &Path, failed: &AtomicBool) -> Gitignore {
    if !path.is_file() {
        return Gitignore::empty();
    }
    let (matcher, error) = Gitignore::new(path);
    if let Some(error) = error {
        eprintln!("failed to read ignore rules {}: {error}", path.display());
        failed.store(true, Ordering::Relaxed);
    }
    matcher
}

// A linked worktree stores .git as a gitdir pointer; its exclude file lives
// in the shared directory named by commondir.
fn git_common_dir(dir: &Path) -> Option<PathBuf> {
    let dot_git = dir.join(".git");
    if dot_git.is_dir() {
        return Some(dot_git);
    }
    if !dot_git.is_file() {
        return None;
    }

    let contents = fs::read_to_string(&dot_git).ok()?;
    let gitdir = contents.lines().next()?.strip_prefix("gitdir: ")?;
    let gitdir = Path::new(gitdir);
    let gitdir = if gitdir.is_absolute() {
        gitdir.to_path_buf()
    } else {
        dir.join(gitdir)
    };
    let commondir = gitdir.join("commondir");
    let Ok(contents) = fs::read_to_string(commondir) else {
        return Some(gitdir);
    };
    let common = Path::new(contents.lines().next()?);
    Some(if common.is_absolute() {
        common.to_path_buf()
    } else {
        gitdir.join(common)
    })
}

fn exclude_matcher(dir: &Path, failed: &AtomicBool) -> Gitignore {
    let Some(git_dir) = git_common_dir(dir) else {
        return Gitignore::empty();
    };
    let path = git_dir.join("info/exclude");
    if !path.is_file() {
        return Gitignore::empty();
    }
    let mut builder = GitignoreBuilder::new(dir);
    if let Some(error) = builder.add(&path) {
        eprintln!("failed to read ignore rules {}: {error}", path.display());
        failed.store(true, Ordering::Relaxed);
    }
    match builder.build() {
        Ok(matcher) => matcher,
        Err(error) => {
            eprintln!("failed to build ignore rules {}: {error}", path.display());
            failed.store(true, Ordering::Relaxed);
            Gitignore::empty()
        }
    }
}

pub(super) fn extend_rules(
    dir: &Path,
    parent: Option<Arc<Rules>>,
    ignore_git: bool,
    failed: &AtomicBool,
) -> Option<Arc<Rules>> {
    let git_root = ignore_git && dir.join(".git").exists();
    let ignore = matcher(&dir.join(".ignore"), failed);
    let gitignore = if ignore_git {
        matcher(&dir.join(".gitignore"), failed)
    } else {
        Gitignore::empty()
    };
    let exclude = if git_root {
        exclude_matcher(dir, failed)
    } else {
        Gitignore::empty()
    };
    if !git_root && ignore.is_empty() && gitignore.is_empty() && exclude.is_empty() {
        return parent;
    }
    Some(Arc::new(Rules {
        parent,
        git_root,
        ignore,
        gitignore,
        exclude,
    }))
}
