mod dir;
mod file;
mod language;

use dir::scan_directory;
use file::{Batch, Stats, Summary, parse_file};
use std::{
    io::IsTerminal,
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{self, Receiver},
    },
    time::Duration,
};

#[argue::parser(name = "tally", about = "Count and inspect a codebase")]
#[derive(Debug)]
struct Args {
    /// Include files ignored by gitignore rules.
    #[flag(short = 'a', long = "all")]
    all: bool,

    /// Number of worker threads. Defaults to up to 4 workers for directories and 1 for a file.
    #[option(short = 'j', long = "threads")]
    threads: Option<usize>,

    /// Print extra diagnostics, including unknown file formats.
    #[flag(short = 'v', long = "verbose")]
    verbose: bool,

    /// Path to tally
    #[positional(default = ".")]
    path: PathBuf,
}

fn main() {
    let args = parse_args();
    let path_is_dir = args.path.is_dir();
    let threads = args.threads.unwrap_or_else(|| default_threads(path_is_dir));
    let verbose = args.verbose;
    let sink = file::Sink::new();
    let progress = std::io::stderr().is_terminal().then(|| {
        let (progress_done, done) = mpsc::channel();
        (progress_done, show_progress(Arc::clone(&sink), done))
    });

    if path_is_dir {
        scan_directory(&args.path, Arc::clone(&sink), !args.all, threads, verbose);
    } else {
        parse_single_file(&args.path, &sink, verbose);
    }

    if let Some((progress_done, progress)) = progress {
        let _ = progress_done.send(());
        progress.join().unwrap();
    }

    let summary = sink.snapshot();
    print_summary(&summary);

    if verbose {
        print_unknown_formats(&summary);
    }
}

fn parse_args() -> Args {
    match Args::parse() {
        Ok(args) => args,
        Err(err) => {
            match &err {
                argue::Error::Help(help) => println!("{help}"),
                _ => eprintln!("{err}"),
            }
            std::process::exit(err.exit_code());
        }
    }
}

fn default_threads(path_is_dir: bool) -> usize {
    if !path_is_dir {
        return 1;
    }

    std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(4)
}

fn parse_single_file(path: &PathBuf, sink: &file::Sink, verbose: bool) {
    let mut batch = Batch::default();
    if let Some(file_stats) = parse_file(path, verbose) {
        batch.add(file_stats);
    }
    sink.record_progress(batch.files());
    sink.add_batch(&mut batch);
}

fn show_progress(sink: Arc<file::Sink>, done: Receiver<()>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut last_files = None;

        loop {
            match done.recv_timeout(Duration::from_millis(250)) {
                Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    let files = sink.files();
                    if last_files == Some(files) {
                        continue;
                    }

                    last_files = Some(files);
                    eprint!("\rprocessed {files} files");
                }
            }
        }

        eprint!("\r{:<24}\r", "");
    })
}

fn print_summary(summary: &Summary) {
    let mut rows = summary_rows(summary);
    let widths = table_widths(&rows, summary.all);

    print_header(widths);
    for (name, stats) in rows.drain(..) {
        print_row(widths, name, stats);
    }
    print_separator(widths);
    print_row(widths, "Total", summary.all);
}

fn summary_rows(summary: &Summary) -> Vec<(&'static str, Stats)> {
    let mut languages = summary.languages.clone();
    languages.sort_by_key(|&(language_id, _)| crate::language::get(language_id).name);

    let mut rows = languages
        .into_iter()
        .map(|(language_id, stats)| (crate::language::get(language_id).name, stats))
        .collect::<Vec<_>>();

    if summary.unknown.files > 0 {
        rows.push(("Unknown", summary.unknown));
    }

    rows
}

#[derive(Clone, Copy)]
struct TableWidths {
    name: usize,
    files: usize,
    lines: usize,
    blanks: usize,
    comments: usize,
    code: usize,
}

fn table_widths(rows: &[(&str, Stats)], total: Stats) -> TableWidths {
    let mut widths = TableWidths {
        name: "Language".len(),
        files: "Files".len(),
        lines: "Lines".len(),
        blanks: "Blank".len(),
        comments: "Comment".len(),
        code: "Code".len(),
    };

    for &(name, stats) in rows.iter().chain([("Total", total)].iter()) {
        widths.name = widths.name.max(name.len());
        widths.files = widths.files.max(digits(stats.files));
        widths.lines = widths.lines.max(digits(stats.lines));
        widths.blanks = widths.blanks.max(digits(stats.blanks));
        widths.comments = widths.comments.max(digits(stats.comments));
        widths.code = widths.code.max(digits(stats.code));
    }

    widths
}

fn digits(number: u64) -> usize {
    number.max(1).ilog10() as usize + 1
}

fn print_header(widths: TableWidths) {
    println!(
        "{:<name$} {:>files$} {:>lines$} {:>blanks$} {:>comments$} {:>code$}",
        "Language",
        "Files",
        "Lines",
        "Blank",
        "Comment",
        "Code",
        name = widths.name,
        files = widths.files,
        lines = widths.lines,
        blanks = widths.blanks,
        comments = widths.comments,
        code = widths.code,
    );
    print_separator(widths);
}

fn print_separator(widths: TableWidths) {
    println!(
        "{:-<name$} {:-<files$} {:-<lines$} {:-<blanks$} {:-<comments$} {:-<code$}",
        "",
        "",
        "",
        "",
        "",
        "",
        name = widths.name,
        files = widths.files,
        lines = widths.lines,
        blanks = widths.blanks,
        comments = widths.comments,
        code = widths.code,
    );
}

fn print_unknown_formats(summary: &Summary) {
    if summary.unknown_formats.is_empty() {
        return;
    }

    eprintln!("\nUnknown file formats:");
    for (format, files) in &summary.unknown_formats {
        eprintln!("  {format:<24} {files:>8}");
    }
}

fn print_row(widths: TableWidths, name: &str, stats: Stats) {
    println!(
        "{:<name_width$} {:>files_width$} {:>lines_width$} {:>blanks_width$} {:>comments_width$} {:>code_width$}",
        name,
        stats.files,
        stats.lines,
        stats.blanks,
        stats.comments,
        stats.code,
        name_width = widths.name,
        files_width = widths.files,
        lines_width = widths.lines,
        blanks_width = widths.blanks,
        comments_width = widths.comments,
        code_width = widths.code,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_threads_uses_one_worker_for_a_file() {
        assert_eq!(default_threads(false), 1);
    }

    #[test]
    fn default_threads_caps_directory_workers() {
        assert!((1..=4).contains(&default_threads(true)));
    }

    #[test]
    fn args_apply_defaults() {
        let args = Args::parse_from(["tally"]).unwrap();

        assert!(!args.all);
        assert!(!args.verbose);
        assert_eq!(args.threads, None);
        assert_eq!(args.path, PathBuf::from("."));
    }

    #[test]
    fn args_parse_flags_options_and_path() {
        let args = Args::parse_from(["tally", "--all", "-v", "-j", "2", "src"]).unwrap();

        assert!(args.all);
        assert!(args.verbose);
        assert_eq!(args.threads, Some(2));
        assert_eq!(args.path, PathBuf::from("src"));
    }

    #[test]
    fn args_report_help() {
        let err = Args::parse_from(["tally", "--help"]).unwrap_err();

        assert_eq!(err, arogant::Error::Help(Args::HELP));
    }
}
