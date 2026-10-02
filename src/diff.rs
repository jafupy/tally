use crate::{file, language, output, result};
use ignore::overrides::Override;
use imara_diff::{Algorithm, Diff, InternedInput};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    io::{self, IsTerminal, Write},
    path::Path,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

#[derive(Clone, Copy, Default, serde::Serialize)]
struct Changes {
    added: result::Stats,
    deleted: result::Stats,
}

type FileCounts = [Option<(&'static str, result::Stats)>; 2];

pub fn count(
    root: &Path,
    reference: &str,
    target: Option<&str>,
    overrides: &Override,
    json: bool,
    threads: usize,
    adaptive_threads: bool,
) -> io::Result<()> {
    if !root.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "--diff requires a directory",
        ));
    }
    let trace_output = crate::trace_output::TraceOutput::current()?;
    let repo = tally_git::Repository::open(root)?;
    let files = repo.files(reference, target, |path| {
        !trace_output.matches_file(&root.join(path))
            && crate::scan::file_is_included(overrides, path)
    })?;
    let changes = count_files(root, &repo, files, threads, adaptive_threads)?;
    let mut languages = HashMap::<&'static str, Changes>::new();
    for [added, deleted] in changes {
        if let Some((name, stats)) = added {
            languages.entry(name).or_default().added += stats;
        }
        if let Some((name, stats)) = deleted {
            languages.entry(name).or_default().deleted += stats;
        }
    }
    let mut rows = languages
        .into_iter()
        .filter(|(_, c)| c.added.lines + c.deleted.lines > 0)
        .collect::<Vec<_>>();
    rows.sort_by(|(an, a), (bn, b)| {
        (b.added.code + b.deleted.code)
            .cmp(&(a.added.code + a.deleted.code))
            .then_with(|| an.cmp(bn))
    });
    let total = rows.iter().fold(Changes::default(), |mut t, (_, c)| {
        t.added += c.added;
        t.deleted += c.deleted;
        t
    });
    if json {
        print_json(&rows, total)
    } else {
        print_table(&rows, total, io::stdout().is_terminal())
    }
}

fn count_files(
    root: &Path,
    repo: &tally_git::Repository,
    files: Vec<tally_git::FilePair>,
    threads: usize,
    adaptive_threads: bool,
) -> io::Result<Vec<FileCounts>> {
    let max_workers = threads.max(1).min(files.len().max(1));
    if max_workers == 1 {
        let mut counts = Vec::new();
        for file in files {
            if let Some(counted) = count_file(repo, file)? {
                counts.push(counted);
            }
        }
        return Ok(counts);
    }
    let pending = AtomicUsize::new(files.len());
    let failed = AtomicBool::new(false);
    let queue = Mutex::new(VecDeque::from(files));
    thread::scope(|scope| {
        let mut handles = Vec::with_capacity(max_workers);
        let spawn_worker = || {
            let queue = &queue;
            let pending = &pending;
            let failed = &failed;
            scope.spawn(move || -> io::Result<Vec<FileCounts>> {
                let repo = tally_git::Repository::open(root).map_err(|error| {
                    failed.store(true, Ordering::Release);
                    error
                })?;
                let mut counts = Vec::new();
                while !failed.load(Ordering::Relaxed) {
                    let job = queue.lock().unwrap().pop_front();
                    let Some(file) = job else { break };
                    match count_file(&repo, file) {
                        Ok(Some(counted)) => counts.push(counted),
                        Ok(None) => {}
                        Err(error) => {
                            failed.store(true, Ordering::Release);
                            return Err(error);
                        }
                    }
                    pending.fetch_sub(1, Ordering::AcqRel);
                }
                Ok(counts)
            })
        };
        handles.push(spawn_worker());
        if !adaptive_threads {
            while handles.len() < max_workers {
                handles.push(spawn_worker());
            }
        }
        while pending.load(Ordering::Acquire) > 0 && !failed.load(Ordering::Acquire) {
            let queued = queue.lock().unwrap().len();
            if handles.len() < max_workers && queued > handles.len() {
                handles.push(spawn_worker());
            }
            thread::park_timeout(Duration::from_micros(100));
        }
        let mut counts = Vec::new();
        let mut error = None;
        for handle in handles {
            match handle.join().unwrap() {
                Ok(worker_counts) => counts.extend(worker_counts),
                Err(worker_error) => error = Some(worker_error),
            }
        }
        error.map_or(Ok(counts), Err)
    })
}

fn count_file(
    repo: &tally_git::Repository,
    file: tally_git::FilePair,
) -> io::Result<Option<FileCounts>> {
    if file.working_tree.is_none() && file.old == file.new {
        return Ok(None);
    }
    let old = file.old.map(|id| repo.blob(id)).transpose()?;
    let new = file.new.map(|id| repo.blob(id)).transpose()?;
    let working = if let Some(path) = file.working_tree.as_ref() {
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_file() => Some(fs::read(path)?),
            _ => None,
        }
    } else {
        None
    };
    let old_bytes = old.as_ref().map(|blob| blob.content()).unwrap_or_default();
    let new_bytes = new
        .as_ref()
        .map(|blob| blob.content())
        .or(working.as_deref())
        .unwrap_or_default();
    if old_bytes == new_bytes || is_binary(old_bytes) || is_binary(new_bytes) {
        return Ok(None);
    }
    let input = InternedInput::new(old_bytes, new_bytes);
    let diff = Diff::compute(Algorithm::Myers, &input);
    let (mut deleted_lines, mut added_lines) = (HashSet::new(), HashSet::new());
    for hunk in diff.hunks() {
        deleted_lines.extend(hunk.before.map(|line| line as usize + 1));
        added_lines.extend(hunk.after.map(|line| line as usize + 1));
    }
    if deleted_lines.is_empty() && added_lines.is_empty() {
        return Ok(None);
    }
    Ok(Some([
        selected_stats(&file.path, new_bytes, &added_lines),
        selected_stats(&file.path, old_bytes, &deleted_lines),
    ]))
}

fn is_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8000).any(|byte| *byte == 0)
}

fn selected_stats(
    path: &Path,
    contents: &[u8],
    lines: &HashSet<usize>,
) -> Option<(&'static str, result::Stats)> {
    if lines.is_empty() {
        return None;
    }
    let Some((language_id, mut stats)) = file::count_selected_contents(path, contents, lines)
    else {
        return None;
    };
    stats.files = 1;
    let name = language_id.map_or("Unknown", |id| language::get(id).name);
    Some((name, stats))
}

fn print_json(rows: &[(&str, Changes)], total: Changes) -> io::Result<()> {
    #[derive(serde::Serialize)]
    struct Lang<'a> {
        language: &'a str,
        #[serde(flatten)]
        changes: Changes,
    }
    #[derive(serde::Serialize)]
    struct Diff<'a> {
        languages: Vec<Lang<'a>>,
        total: Changes,
    }
    let value = Diff {
        languages: rows
            .iter()
            .map(|&(language, changes)| Lang { language, changes })
            .collect(),
        total,
    };
    writeln!(
        io::stdout().lock(),
        "{}",
        serde_json::to_string_pretty(&value).unwrap()
    )
}
fn pairs(c: Changes) -> [String; 5] {
    let p = |a, d| {
        if a == d {
            format!("±{}", output::format_number(a))
        } else {
            format!(
                "+{}/-{}",
                output::format_number(a),
                output::format_number(d)
            )
        }
    };
    [
        p(c.added.files, c.deleted.files),
        p(c.added.lines, c.deleted.lines),
        p(c.added.blanks, c.deleted.blanks),
        p(c.added.comments, c.deleted.comments),
        p(c.added.code, c.deleted.code),
    ]
}
fn widths(rows: &[(&str, Changes)], total: Changes) -> [usize; 6] {
    let mut w = [8, 5, 5, 5, 7, 4];
    for &(n, c) in rows.iter().chain(std::iter::once(&("Total", total))) {
        w[0] = w[0].max(n.len());
        for (i, p) in pairs(c).iter().enumerate() {
            w[i + 1] = w[i + 1].max(p.chars().count())
        }
    }
    w
}
fn print_table(rows: &[(&str, Changes)], total: Changes, color: bool) -> io::Result<()> {
    let w = widths(rows, total);
    let mut out = io::stdout().lock();
    styled(
        &mut out,
        &format!(
            "{:<a$} {:>b$} {:>c$} {:>d$} {:>e$} {:>f$}",
            "Language",
            "Files",
            "Lines",
            "Blank",
            "Comment",
            "Code",
            a = w[0],
            b = w[1],
            c = w[2],
            d = w[3],
            e = w[4],
            f = w[5]
        ),
        color,
        "\x1b[1;36m",
    )?;
    separator(&mut out, w, color)?;
    for &(n, c) in rows {
        row(&mut out, n, c, w, color, false)?
    }
    separator(&mut out, w, color)?;
    row(&mut out, "Total", total, w, color, true)
}
fn row(
    out: &mut impl Write,
    name: &str,
    c: Changes,
    w: [usize; 6],
    color: bool,
    total: bool,
) -> io::Result<()> {
    if color && total {
        write!(out, "\x1b[1m")?
    }
    if color {
        write!(out, "\x1b[34m{name:<x$}\x1b[0m", x = w[0])?
    } else {
        write!(out, "{name:<x$}", x = w[0])?
    }
    for (p, width) in pairs(c).iter().zip(&w[1..]) {
        write!(out, " {:>x$}", "", x = width - p.chars().count())?;
        if color && p == "±0" {
            write!(out, "{}{p}\x1b[0m", output::DIM_STYLE)?
        } else if color && p.starts_with('±') {
            write!(out, "\x1b[37m{p}\x1b[0m")?
        } else if color {
            let (a, d) = p.split_once('/').unwrap();
            write!(
                out,
                "\x1b[32m{a}\x1b[0m{}/\x1b[0m\x1b[31m{d}\x1b[0m",
                output::DIM_STYLE
            )?
        } else {
            write!(out, "{p}")?
        }
    }
    if color && total {
        write!(out, "\x1b[0m")?
    }
    writeln!(out)
}
fn separator(out: &mut impl Write, w: [usize; 6], color: bool) -> io::Result<()> {
    styled(
        out,
        &"─".repeat(w.iter().sum::<usize>() + 5),
        color,
        output::DIM_STYLE,
    )
}
fn styled(out: &mut impl Write, line: &str, color: bool, style: &str) -> io::Result<()> {
    if color {
        writeln!(out, "{style}{line}\x1b[0m")
    } else {
        writeln!(out, "{line}")
    }
}
