//! `aralo`: the core from a terminal. It checks a library, lists it, moves
//! snippets in and out of other tools, and types into a text field that exists
//! only in memory, so the whole expansion path runs on a machine with no
//! window server (CI on Linux included).

// A command-line tool's job is to print. It never sees real keystrokes: the
// only text it handles is what the user passed on the command line.
#![allow(clippy::print_stdout, clippy::print_stderr)]

mod ai;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use aralo_core::{
    Core, ExportOptions, Field, Format, ImportOptions, ImportReport, Issue, MacroPolicy, Outcome,
    Query, SearchHit, Simulator,
};
use clap::{Parser, Subcommand, ValueEnum};

/// Anything that stops a command: a core error, a file that would not write,
/// or a flag that makes no sense. `main` prints it and exits 2.
type Failure = Box<dyn std::error::Error>;

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
    /// Find snippets by abbreviation, label, tag, group or body text; exit 1
    /// if nothing matches. Left without a query it lists the whole library.
    Search {
        library: PathBuf,
        /// What to look for. Short fields are matched loosely, so `bregs`
        /// finds "Best regards"; a body has to hold the text as it is typed.
        query: Option<String>,
        /// Only this group and the groups inside it, `Work/Email` style
        #[arg(long)]
        group: Option<String>,
        /// Only snippets carrying this tag
        #[arg(long)]
        tag: Option<String>,
        /// Leave out snippets that are switched off
        #[arg(long)]
        enabled: bool,
        /// Show at most this many
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Type text into an imaginary text field and print what ends up in it
    Type {
        library: PathBuf,
        /// What to type. `\n` and `\t` stand for Return and Tab.
        text: String,
        /// Bundle ID of the app that has focus, for scoped groups
        #[arg(long, default_value = "com.apple.TextEdit")]
        app: String,
    },
    /// Type one abbreviation and print what it expands to; exit 1 if nothing
    /// expands it. A snippet that waits for a delimiter gets a space, so the
    /// output keeps that space when the snippet keeps it.
    Expand {
        library: PathBuf,
        /// The abbreviation, exactly as you would type it
        abbr: String,
        /// Bundle ID of the app that has focus, for scoped groups
        #[arg(long, default_value = "com.apple.TextEdit")]
        app: String,
    },
    /// Read snippets from another expander into a library; exit 1 if any of
    /// them needs an edit or was skipped
    Import {
        /// The file to read: a TextExpander export, a CSV, JSON or YAML
        source: PathBuf,
        library: PathBuf,
        /// What the source is. Worked out from the file when left out.
        #[arg(long, value_enum)]
        format: Option<SourceFormat>,
        /// Put everything under this group, `Work/Email` style
        #[arg(long)]
        into: Option<String>,
        /// What to do with TextExpander macros in a body
        #[arg(long, value_enum, default_value_t = Macros::Auto)]
        macros: Macros,
        /// Say what would happen and write nothing
        #[arg(long)]
        dry_run: bool,
        /// How to print the report
        #[arg(long, value_enum, default_value_t = ReportStyle::Text)]
        report: ReportStyle,
    },
    /// Write a library out as one interchange file
    Export {
        library: PathBuf,
        /// Where to write it; `-` writes to standard output
        out: PathBuf,
        /// Worked out from the file name when left out
        #[arg(long, value_enum)]
        format: Option<ExportFormat>,
        /// Export only this group and the groups inside it
        #[arg(long)]
        group: Option<String>,
    },
    /// Set up AI: the switch, provider profiles and their keys
    Ai {
        #[command(subcommand)]
        command: ai::AiCommand,
    },
}

/// The formats an import reads. TextExpander is read-only: Aralo does not
/// write those files, so it is not in [`ExportFormat`].
#[derive(Debug, Clone, Copy, ValueEnum)]
enum SourceFormat {
    Textexpander,
    Csv,
    Json,
    Yaml,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ExportFormat {
    Json,
    Yaml,
    Csv,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Macros {
    /// Convert them where a body plainly carries them
    Auto,
    /// Convert them everywhere, even where `%d` may be a format string
    Convert,
    /// Convert none of them; the body is literal text
    Literal,
    /// The body is an Aralo template already
    Template,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ReportStyle {
    Text,
    Json,
}

impl SourceFormat {
    fn format(self) -> Format {
        match self {
            SourceFormat::Textexpander => Format::TextExpander,
            SourceFormat::Csv => Format::Csv,
            SourceFormat::Json => Format::Json,
            SourceFormat::Yaml => Format::Yaml,
        }
    }
}

impl ExportFormat {
    fn format(self) -> Format {
        match self {
            ExportFormat::Json => Format::Json,
            ExportFormat::Yaml => Format::Yaml,
            ExportFormat::Csv => Format::Csv,
        }
    }
}

impl Macros {
    fn policy(self) -> MacroPolicy {
        match self {
            Macros::Auto => MacroPolicy::Auto,
            Macros::Convert => MacroPolicy::Convert,
            Macros::Literal => MacroPolicy::Literal,
            Macros::Template => MacroPolicy::Template,
        }
    }
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

fn run(command: Command) -> Result<ExitCode, Failure> {
    match command {
        Command::Ai { command } => ai::run(command),
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
        Command::Search {
            library,
            query,
            group,
            tag,
            enabled,
            limit,
        } => {
            let core = Core::open_read_only(&library)?;
            let hits = core.search(&Query {
                text: query.unwrap_or_default(),
                group: group.as_deref().map(|group| group_path(Some(group))),
                tag,
                enabled_only: enabled,
                kind: None,
                // One more than asked for, so the footer can say whether
                // anything was left out rather than guessing from the count.
                limit: Some(limit.saturating_add(1)),
            });
            for hit in hits.iter().take(limit) {
                print_hit(hit);
            }
            if hits.len() > limit {
                println!("… more; raise --limit to see them");
            }
            Ok(if hits.is_empty() {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            })
        }
        Command::Type { library, text, app } => {
            let core = Core::open_read_only(&library)?;
            let typed = text.replace("\\n", "\n").replace("\\t", "\t");
            let mut field = Simulator::new(&core, &app);
            field.type_str(&typed);
            println!("{}", field.text());
            Ok(ExitCode::SUCCESS)
        }
        Command::Expand { library, abbr, app } => {
            let core = Core::open_read_only(&library)?;
            let mut field = Simulator::new(&core, &app);
            field.type_str(&abbr);
            // A snippet that waits for a delimiter has not fired yet. The
            // space is kept only if the snippet keeps it, so what is printed
            // is still exactly what would be typed.
            if field.expansions() == 0 {
                field.type_str(" ");
            }
            if field.expansions() == 0 {
                eprintln!(
                    "aralo: nothing in {} expands {abbr:?}",
                    core.root().display()
                );
                return Ok(ExitCode::FAILURE);
            }
            println!("{}", field.text());
            Ok(ExitCode::SUCCESS)
        }
        Command::Import {
            source,
            library,
            format,
            into,
            macros,
            dry_run,
            report,
        } => {
            let options = ImportOptions {
                format: format.map(SourceFormat::format),
                into: group_path(into.as_deref()),
                macros: macros.policy(),
                dry_run,
            };
            // A dry run reads the library and leaves the folder alone, so it
            // will not create one either.
            let mut core = if dry_run {
                Core::open_read_only(&library)?
            } else {
                Core::open_without_starter(&library)?
            };
            let imported = core.import(&source, &options)?;
            match report {
                ReportStyle::Text => print!("{imported}"),
                ReportStyle::Json => println!("{}", serde_json::to_string_pretty(&imported)?),
            }
            Ok(attention(&imported))
        }
        Command::Export {
            library,
            out,
            format,
            group,
        } => {
            let core = Core::open_read_only(&library)?;
            let group = group_path(group.as_deref());
            let format = match format {
                Some(format) => format.format(),
                None => format_for(&out)?,
            };
            let bytes = core.export(&ExportOptions {
                format,
                group: group.clone(),
            })?;
            if out == Path::new("-") {
                std::io::stdout().write_all(&bytes)?;
                return Ok(ExitCode::SUCCESS);
            }
            std::fs::write(&out, &bytes)
                .map_err(|error| format!("cannot write {}: {error}", out.display()))?;
            let count = core
                .snippets()
                .iter()
                .filter(|snippet| snippet.group.starts_with(&group))
                .count();
            println!(
                "{}: {count} {} as {format}",
                out.display(),
                if count == 1 { "snippet" } else { "snippets" }
            );
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// One search hit: the snippet, and why it matched when that is not already
/// plain from its name.
fn print_hit(hit: &SearchHit) {
    let state = if hit.enabled { "" } else { " (off)" };
    // The name and the abbreviations are already on the line; saying "abbr:
    // ;follow" after them tells the reader nothing.
    let why = if hit.text == hit.name || hit.abbr.contains(&hit.text) {
        String::new()
    } else {
        format!(
            "  {}: {}",
            field_name(hit.field),
            hit.text.replace('\n', " ")
        )
    };
    println!(
        "{:<16} {}{state}{why}  [{}]",
        hit.abbr.join(" "),
        hit.name,
        hit.path.display()
    );
}

fn field_name(field: Field) -> &'static str {
    match field {
        Field::Abbreviation => "abbr",
        Field::Label => "label",
        Field::Tag => "tag",
        Field::Group => "group",
        Field::Body => "body",
    }
}

/// `Work/Email` as the group path the library uses.
fn group_path(text: Option<&str>) -> Vec<String> {
    text.into_iter()
        .flat_map(|text| text.split('/'))
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The format an export file name asks for.
fn format_for(path: &Path) -> Result<Format, Failure> {
    let extension = path
        .extension()
        .map(|extension| extension.to_string_lossy().to_ascii_lowercase());
    match extension.as_deref() {
        Some("json") => Ok(Format::Json),
        Some("yaml" | "yml") => Ok(Format::Yaml),
        Some("csv") => Ok(Format::Csv),
        _ => Err(format!(
            "cannot tell what format {} should be; say which with --format",
            path.display()
        )
        .into()),
    }
}

/// Exit 1 when the import left something for a human to look at.
fn attention(report: &ImportReport) -> ExitCode {
    if report.count(Outcome::NeedsEdit) + report.count(Outcome::Skipped) == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
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
        Issue::ConflictCopy { original } => (
            Level::Warning,
            format!(
                "a sync conflict copy of {}; the app merges it when it can, or keep one and delete the other",
                original.display()
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
