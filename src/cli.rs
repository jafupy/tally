use std::path::PathBuf;

#[cfg(feature = "debug")]
#[argue::parser(
    name = "tally",
    about = "Count and inspect a codebase",
    long_about = "Diff usage: tally [path] --diff [revision] [--diff revision]\nOne revision compares with the working tree; two revisions compare with each other. The path defaults to . and the first revision to HEAD. Put an explicit path before --diff."
)]
#[derive(Debug)]
pub(crate) struct Args {
    /// Print the version and check GitHub for updates.
    #[flag(short = 'V', long = "version")]
    pub(crate) version: bool,

    /// Include files ignored by gitignore rules.
    #[flag(short = 'a', long = "all")]
    pub(crate) all: bool,

    /// Number of worker threads. Defaults to adaptive scaling for directories and 1 for a file.
    #[option(short = 'j', long = "threads")]
    pub(crate) threads: Option<usize>,

    /// Print scan diagnostics (summary by default); use --debug=max for a full trace.
    #[option(short = 'd', long = "debug", default = DebugLevel::Off, optional = "summary", equals = true, value_name = "summary|max")]
    pub(crate) debug: DebugLevel,

    /// Output results as JSON.
    #[flag(long = "json")]
    pub(crate) json: bool,

    /// Add per-file Blank, Comment, and Code columns.
    /// Use min, max, mean, median, sd, iqr, variance, p0..p100, or NAME=EXPR.
    /// Bare -x selects min, max, median, and sd.
    #[option(
        short = 'x',
        long = "extended",
        optional = "default",
        equals = true,
        value_name = "STAT"
    )]
    pub(crate) extended: Vec<String>,

    /// Compare a git revision with the working tree, or repeat to compare two revisions.
    #[option(long = "diff", optional = "HEAD", equals = true, value_name = "REV")]
    pub(crate) diff: Vec<String>,

    /// Count only files known to git.
    #[flag(long = "tracked")]
    pub(crate) tracked: bool,

    /// Include paths matching this glob. May be repeated.
    #[option(long = "include")]
    pub(crate) include: Vec<String>,

    /// Exclude paths matching this glob. May be repeated.
    #[option(long = "exclude")]
    pub(crate) exclude: Vec<String>,

    /// Path to tally
    #[positional(default = ".")]
    pub(crate) path: PathBuf,
}

#[cfg(not(feature = "debug"))]
#[argue::parser(
    name = "tally",
    about = "Count and inspect a codebase",
    long_about = "Diff usage: tally [path] --diff [revision] [--diff revision]\nOne revision compares with the working tree; two revisions compare with each other. The path defaults to . and the first revision to HEAD. Put an explicit path before --diff."
)]
#[derive(Debug)]
pub(crate) struct Args {
    /// Print the version and check GitHub for updates.
    #[flag(short = 'V', long = "version")]
    pub(crate) version: bool,

    /// Include files ignored by gitignore rules.
    #[flag(short = 'a', long = "all")]
    pub(crate) all: bool,

    /// Number of worker threads. Defaults to adaptive scaling for directories and 1 for a file.
    #[option(short = 'j', long = "threads")]
    pub(crate) threads: Option<usize>,

    /// Output results as JSON.
    #[flag(long = "json")]
    pub(crate) json: bool,

    /// Add per-file Blank, Comment, and Code columns.
    /// Use min, max, mean, median, sd, iqr, variance, p0..p100, or NAME=EXPR.
    /// Bare -x selects min, max, median, and sd.
    #[option(
        short = 'x',
        long = "extended",
        optional = "default",
        equals = true,
        value_name = "STAT"
    )]
    pub(crate) extended: Vec<String>,

    /// Compare a git revision with the working tree, or repeat to compare two revisions.
    #[option(long = "diff", optional = "HEAD", equals = true, value_name = "REV")]
    pub(crate) diff: Vec<String>,

    /// Count only files known to git.
    #[flag(long = "tracked")]
    pub(crate) tracked: bool,

    /// Include paths matching this glob. May be repeated.
    #[option(long = "include")]
    pub(crate) include: Vec<String>,

    /// Exclude paths matching this glob. May be repeated.
    #[option(long = "exclude")]
    pub(crate) exclude: Vec<String>,

    /// Path to tally
    #[positional(default = ".")]
    pub(crate) path: PathBuf,
}

#[cfg(feature = "debug")]
#[argue::args]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DebugLevel {
    Off,
    Summary,
    Max,
}

impl Args {
    pub(crate) fn debug_enabled(&self) -> bool {
        #[cfg(feature = "debug")]
        {
            self.debug != DebugLevel::Off
        }
        #[cfg(not(feature = "debug"))]
        {
            false
        }
    }

    #[cfg(feature = "debug")]
    pub(crate) fn trace_requested(&self) -> bool {
        self.debug == DebugLevel::Max
    }
}

pub(crate) fn parse() -> Args {
    let mut argv = std::env::args_os().collect::<Vec<_>>();
    if let Some(position) = argv.iter().position(|arg| arg == "-")
        && !argv[..position].iter().any(|arg| arg == "--")
        && (position == 1
            || (position + 1 == argv.len()
                && !matches!(
                    argv.get(position.wrapping_sub(1))
                        .and_then(|arg| arg.to_str()),
                    Some("-j" | "--threads" | "--diff" | "--include" | "--exclude")
                )))
    {
        argv.remove(position);
        argv.push("--".into());
        argv.push("-".into());
    }
    // argue's optional values only accept explicit revisions with `=`.
    // Join space-separated revisions without touching other option values or `--`.
    let mut position = 1;
    while position < argv.len() {
        if let Some(value) = argv[position]
            .to_str()
            .and_then(|arg| arg.strip_prefix("-x="))
        {
            argv[position] = format!("--extended={value}").into();
        }
        match argv[position].to_str() {
            Some("--") => break,
            Some("-j" | "--threads" | "--include" | "--exclude") => position += 2,
            Some("--diff") => {
                if argv.get(position + 1).is_some_and(|value| {
                    value == "-" || !value.as_encoded_bytes().starts_with(b"-")
                }) {
                    let revision = argv.remove(position + 1);
                    argv[position].push("=");
                    argv[position].push(revision);
                }
                position += 1;
            }
            _ => position += 1,
        }
    }
    match Args::parse_from(argv) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_apply_defaults() {
        let args = Args::parse_from(["tally"]).unwrap();

        assert!(!args.all);
        #[cfg(feature = "debug")]
        assert_eq!(args.debug, DebugLevel::Off);
        assert!(!args.json);
        assert!(!args.version);
        assert!(!args.tracked);
        assert!(args.diff.is_empty());
        assert!(args.include.is_empty());
        assert!(args.exclude.is_empty());
        assert_eq!(args.threads, None);
        assert_eq!(args.path, PathBuf::from("."));
    }

    #[test]
    fn args_accept_repeated_filters() {
        let args = Args::parse_from([
            "tally",
            "--include",
            "*.rs",
            "--exclude",
            "tests/**",
            "--exclude",
            "*.ts",
        ])
        .unwrap();
        assert_eq!(args.include, ["*.rs"]);
        assert_eq!(args.exclude, ["tests/**", "*.ts"]);
    }

    #[cfg(feature = "debug")]
    #[test]
    fn args_parse_flags_options_and_path() {
        let args = Args::parse_from(["tally", "--all", "--json", "-d", "-j", "2", "src"]).unwrap();

        assert!(args.all);
        assert_eq!(args.debug, DebugLevel::Summary);
        assert!(args.json);
        assert_eq!(args.threads, Some(2));
        assert_eq!(args.path, PathBuf::from("src"));
    }

    #[test]
    fn args_report_help() {
        let err = Args::parse_from(["tally", "--help"]).unwrap_err();

        assert_eq!(err, argue::Error::Help(Args::HELP));
    }

    #[test]
    fn args_parse_version() {
        let args = Args::parse_from(["tally", "--version"]).unwrap();

        assert!(args.version);
    }

    #[test]
    fn args_accept_stdin_marker() {
        let args = Args::parse_from(["tally", "--", "-"]).unwrap();

        assert_eq!(args.path, PathBuf::from("-"));
    }
}
