use crate::{file, language, output};
use ignore::overrides::Override;
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Clone, Copy, Default, serde::Serialize)]
struct Changes {
    added: file::Stats,
    deleted: file::Stats,
}
struct ChangedFile {
    path: PathBuf,
    status: u8,
}

pub fn count(root: &Path, reference: &str, overrides: &Override, json: bool) -> io::Result<()> {
    if !root.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "--diff requires a directory",
        ));
    }
    let prefix = repo_prefix(root)?;
    let mut languages = HashMap::<&'static str, Changes>::new();
    for changed in changed_files(root, reference, overrides)? {
        let (deleted_lines, added_lines) = line_numbers(&patch(root, reference, &changed.path)?)?;
        let repo_path = prefix.join(&changed.path);
        let old_regular =
            changed.status != b'A' && revision_is_regular(root, reference, &changed.path)?;
        let new_regular = changed.status != b'D'
            && fs::symlink_metadata(root.join(&changed.path))
                .is_ok_and(|meta| meta.file_type().is_file());
        if !old_regular && !new_regular {
            continue;
        }
        let old = if !old_regular {
            Vec::new()
        } else {
            revision_file(root, reference, &repo_path)?
        };
        let new = if !new_regular {
            Vec::new()
        } else {
            fs::read(root.join(&changed.path))?
        };
        add_selected(&mut languages, &changed.path, &new, &added_lines, true);
        add_selected(&mut languages, &changed.path, &old, &deleted_lines, false);
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

fn changed_files(
    root: &Path,
    reference: &str,
    overrides: &Override,
) -> io::Result<Vec<ChangedFile>> {
    let out = git(
        root,
        &[
            "diff",
            "--name-status",
            "-z",
            "--no-renames",
            "--relative",
            "--end-of-options",
            reference,
            "--",
        ],
    )?;
    let mut fields = out.split(|b| *b == 0).filter(|p| !p.is_empty());
    let mut files = Vec::new();
    while let (Some(status), Some(path)) = (fields.next(), fields.next()) {
        let path = git_path(path);
        if crate::file_is_included(overrides, &path) {
            files.push(ChangedFile {
                path,
                status: status[0],
            });
        }
    }
    Ok(files)
}
fn patch(root: &Path, reference: &str, path: &Path) -> io::Result<Vec<u8>> {
    output(
        Command::new("git")
            .args([
                "diff",
                "--no-color",
                "--no-ext-diff",
                "--unified=0",
                "--no-renames",
                "--end-of-options",
                reference,
                "--",
            ])
            .arg(path)
            .current_dir(root),
    )
}
fn line_numbers(patch: &[u8]) -> io::Result<(HashSet<usize>, HashSet<usize>)> {
    let (mut old, mut new, mut dels, mut adds, mut hunk) =
        (0, 0, HashSet::new(), HashSet::new(), false);
    for line in patch.split(|b| *b == b'\n') {
        if line.starts_with(b"@@ ") {
            let s = std::str::from_utf8(line).map_err(io::Error::other)?;
            let mut r = s.split_ascii_whitespace().skip(1);
            old = start(r.next(), '-')?;
            new = start(r.next(), '+')?;
            hunk = true;
        } else if hunk {
            match line.first() {
                Some(b'-') => {
                    dels.insert(old);
                    old += 1
                }
                Some(b'+') => {
                    adds.insert(new);
                    new += 1
                }
                Some(b' ') => {
                    old += 1;
                    new += 1
                }
                Some(b'\\') | None => {}
                _ => hunk = false,
            }
        }
    }
    Ok((dels, adds))
}
fn start(range: Option<&str>, prefix: char) -> io::Result<usize> {
    range
        .and_then(|r| r.strip_prefix(prefix))
        .and_then(|r| r.split(',').next())
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid git diff hunk"))
}
fn add_selected(
    languages: &mut HashMap<&'static str, Changes>,
    path: &Path,
    contents: &[u8],
    lines: &HashSet<usize>,
    added: bool,
) {
    if lines.is_empty() {
        return;
    }
    let Some((language_id, mut stats)) = file::count_selected_contents(path, contents, lines)
    else {
        return;
    };
    stats.files = 1;
    let name = language_id.map_or("Unknown", |id| language::get(id).name);
    let changes = languages.entry(name).or_default();
    if added {
        changes.added += stats;
    } else {
        changes.deleted += stats;
    }
}
fn repo_prefix(root: &Path) -> io::Result<PathBuf> {
    let mut out = git(root, &["rev-parse", "--show-prefix"])?;
    while out.last().is_some_and(|byte| byte.is_ascii_whitespace()) {
        out.pop();
    }
    Ok(git_path(&out))
}
fn revision_is_regular(root: &Path, reference: &str, path: &Path) -> io::Result<bool> {
    let out = output(
        Command::new("git")
            .args(["ls-tree", "-z", "--end-of-options", reference, "--"])
            .arg(path)
            .current_dir(root),
    )?;
    Ok(out.starts_with(b"100"))
}
fn revision_file(root: &Path, reference: &str, path: &Path) -> io::Result<Vec<u8>> {
    let spec = revision_spec(reference, path);
    output(
        Command::new("git")
            .args(["show", "--no-ext-diff", "--end-of-options"])
            .arg(spec)
            .current_dir(root),
    )
}
#[cfg(unix)]
fn revision_spec(reference: &str, path: &Path) -> std::ffi::OsString {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    let mut bytes = reference.as_bytes().to_vec();
    bytes.push(b':');
    bytes.extend_from_slice(path.as_os_str().as_bytes());
    std::ffi::OsString::from_vec(bytes)
}
#[cfg(not(unix))]
fn revision_spec(reference: &str, path: &Path) -> std::ffi::OsString {
    format!("{reference}:{}", path.to_string_lossy()).into()
}
#[cfg(unix)]
fn git_path(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    PathBuf::from(std::ffi::OsString::from_vec(bytes.to_vec()))
}
#[cfg(not(unix))]
fn git_path(bytes: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
}
fn git(root: &Path, args: &[&str]) -> io::Result<Vec<u8>> {
    output(Command::new("git").args(args).current_dir(root))
}
fn output(command: &mut Command) -> io::Result<Vec<u8>> {
    let o = command.output()?;
    if o.status.success() {
        Ok(o.stdout)
    } else {
        Err(io::Error::other(
            String::from_utf8_lossy(&o.stderr).trim().to_owned(),
        ))
    }
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
