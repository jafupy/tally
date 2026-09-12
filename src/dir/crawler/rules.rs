use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub(super) struct Rules {
    parent: Option<Arc<Rules>>,
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
            rules = current.parent.as_deref();
        }
        rules = Some(self);
        while let Some(current) = rules {
            let matched = current.exclude.matched(path, is_dir);
            if !matched.is_none() {
                return matched.is_ignore();
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

fn exclude_matcher(dir: &Path, failed: &AtomicBool) -> Gitignore {
    let path = dir.join(".git/info/exclude");
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
    let ignore = matcher(&dir.join(".ignore"), failed);
    let gitignore = if ignore_git {
        matcher(&dir.join(".gitignore"), failed)
    } else {
        Gitignore::empty()
    };
    let exclude = if ignore_git {
        exclude_matcher(dir, failed)
    } else {
        Gitignore::empty()
    };
    if ignore.is_empty() && gitignore.is_empty() && exclude.is_empty() {
        return parent;
    }
    Some(Arc::new(Rules {
        parent,
        ignore,
        gitignore,
        exclude,
    }))
}
