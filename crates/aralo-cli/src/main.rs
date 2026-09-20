//! `aralo`: the core from a terminal. It checks a library, lists it, and types
//! into a text field that exists only in memory, so the whole expansion path
//! runs on a machine with no window server (CI on Linux included).

// A command-line tool's job is to print. It never sees real keystrokes: the
// only text it handles is what the user passed on the command line.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::path::PathBuf;
use std::process::ExitCode;

use aralo_core::{Core, CoreError, Issue, Simulator};
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "aralo",
    version,
    about = "Check and try an Aralo snippet library"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create a library folder with the starter snippets
    Init { library: PathBuf },
    /// Report every file that does not load; exit 1 if any is broken
    Validate { library: PathBuf },
    /// List abbreviations and the snippets they expand
    List { library: PathBuf },
    /// Type text into an imaginary text field and print what ends up in it
    Type {
        library: PathBuf,
        /// What to type. `\n` and `\t` stand for Return and Tab.
        text: String,
        /// Bundle ID of the app that has focus, for scoped groups
        #[arg(long, default_value = "com.apple.TextEdit")]
        app: String,
    },
}

fn main() -> ExitCode {
    match run(Cli::parse().command) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("aralo: {error}");
            ExitCode::from(2)
        }
    }
}

fn run(command: Command) -> Result<ExitCode, CoreError> {
    match command {
        Command::Init { library } => {
            let core = Core::open(&library)?;
            println!(
                "{}: {} snippets ({} starter files written)",
                core.root().display(),
                core.snippets().len(),
                core.starter_files_written()
            );
            Ok(ExitCode::SUCCESS)
        }
        Command::Validate { library } => {
            let core = Core::open_read_only(&library)?;
            let mut broken = 0;
            for diagnostic in core.diagnostics() {
                let (level, message) = describe(&diagnostic.issue);
                if level == Level::Error {
                    broken += 1;
                }
                println!("{}: {}: {message}", diagnostic.path.display(), level.name());
            }
            println!("{} snippets, {broken} broken files", core.snippets().len());
            Ok(if broken == 0 {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        Command::List { library } => {
            let core = Core::open_read_only(&library)?;
            for snippet in core.snippets() {
                let front = &snippet.file.front;
                let state = if snippet.settings.enabled {
                    ""
                } else {
                    " (off)"
                };
                println!(
                    "{:<16} {}{state}  [{}]",
                    front.abbr.join(" "),
                    if front.label.is_empty() {
                        "-"
                    } else {
                        &front.label
                    },
                    snippet.path.display()
                );
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Type { library, text, app } => {
            let core = Core::open_read_only(&library)?;
            let typed = text.replace("\\n", "\n").replace("\\t", "\t");
            let mut field = Simulator::new(&core, &app);
            field.type_str(&typed);
            println!("{}", field.text());
            Ok(ExitCode::SUCCESS)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Level {
    Error,
    Warning,
    Note,
}

impl Level {
    fn name(self) -> &'static str {
        match self {
            Level::Error => "error",
            Level::Warning => "warning",
            Level::Note => "note",
        }
    }
}

fn describe(issue: &Issue) -> (Level, String) {
    match issue {
        Issue::Unreadable(why) => (Level::Error, format!("cannot be read: {why}")),
        Issue::Invalid(why) => (Level::Error, why.clone()),
        Issue::TooLarge { bytes } => (
            Level::Error,
            format!("{bytes} bytes is too large for a snippet"),
        ),
        Issue::DuplicateId { first } => (
            Level::Error,
            format!(
                "has the same id as {}; this file is ignored",
                first.display()
            ),
        ),
        Issue::AbbreviationRejected {
            abbreviation,
            reason,
        } => (
            Level::Error,
            format!("abbreviation {abbreviation:?} cannot be used: {reason}"),
        ),
        Issue::TooDeep => (Level::Error, "folders nested too deeply; not loaded".into()),
        Issue::SymlinkedFolder => (
            Level::Warning,
            "symbolic links to folders are not followed".into(),
        ),
        Issue::UnsupportedKind(kind) => (
            Level::Warning,
            format!("{kind:?} snippets do not expand in this version yet"),
        ),
        Issue::MissingId => (
            Level::Note,
            "has no id yet; Aralo adds one when it saves the file".into(),
        ),
        Issue::NotASnippet => (Level::Note, "no front matter, so not a snippet".into()),
    }
}
