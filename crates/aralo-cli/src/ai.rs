//! `aralo ai`: the AI switch, profiles and their keys, from a terminal. It
//! edits the same `profiles.toml` and keychain items as the app's settings.
//!
//! A key is never an argument, where it would land in the shell's history
//! and in the process list. It comes from an environment variable named with
//! `--key-env`, or from standard input with `--key-stdin`.

use std::io::{BufRead, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use aralo_core::ai::{
    builtin_commands, placeholder_changes, profiles_path, provider_preset, AdapterKind, AiSettings,
    AiSwitches, Authoring, BlockRequest, Capabilities, Check, Command, KeyChange, ProfileDraft,
    SavedProfile, Secret, PROVIDER_PRESETS,
};
use aralo_core::diff::Change;
use aralo_core::{Core, Model};
use clap::{Args, Subcommand, ValueEnum};

use crate::Failure;

#[derive(Debug, Subcommand)]
pub enum AiCommand {
    /// Show the switch, local-only mode and the profiles
    Status,
    /// Turn AI on. Nothing is sent anywhere until it is.
    On,
    /// Turn AI off
    Off,
    /// Allow only model servers on this machine, or any again
    LocalOnly { state: OnOff },
    /// List the providers `add --preset` knows, with where to make a key
    Presets,
    /// Save a new profile
    Add {
        /// Defaults to the preset's name
        name: Option<String>,
        /// Fill in the endpoint and model for a known provider
        #[arg(long)]
        preset: Option<String>,
        /// Up to and including the API version, such as https://api.openai.com/v1
        #[arg(long)]
        base_url: Option<String>,
        /// The model to use when a feature names none
        #[arg(long)]
        model: Option<String>,
        /// A header to send with every request, `Name=value`; repeat for more
        #[arg(long = "header", value_name = "NAME=VALUE")]
        headers: Vec<String>,
        #[command(flatten)]
        key: KeySource,
        /// Make it the default profile
        #[arg(long)]
        default: bool,
    },
    /// Change a saved profile
    Edit {
        name: String,
        #[arg(long)]
        rename: Option<String>,
        #[arg(long)]
        base_url: Option<String>,
        #[arg(long)]
        model: Option<String>,
        /// Replaces the saved headers; repeat for more
        #[arg(long = "header", value_name = "NAME=VALUE")]
        headers: Vec<String>,
        /// Send no extra headers
        #[arg(long, conflicts_with = "headers")]
        clear_headers: bool,
        #[command(flatten)]
        key: KeySource,
        /// Delete the profile's key from the keychain
        #[arg(long, conflicts_with_all = ["key_env", "key_stdin"])]
        remove_key: bool,
    },
    /// Delete a profile and its key
    Remove { name: String },
    /// Make a profile the one features use when they name none
    Default { name: String },
    /// Send one short chat to show the key and the model work
    Test {
        /// Defaults to the default profile
        name: Option<String>,
    },
    /// Find out what a profile's endpoint can do, and keep the answer
    Probe { name: Option<String> },
    /// List the models a profile's endpoint serves
    Models { name: Option<String> },
    /// Look for model servers running on this machine
    Detect,
    /// Commands on selected text: the built-in ones and a library's
    #[command(subcommand)]
    Command(CommandAction),
    /// The snippet editor's AI actions: run one on standard input, as if it
    /// were the body, and print what would replace it
    Write {
        action: WriteAction,
        /// For `translate`: the language, in words, such as German
        #[arg(long)]
        language: Option<String>,
        /// For `draft`: the snippet's label. Nothing is read from standard input
        #[arg(long, default_value = "")]
        label: String,
        /// For `draft`: what the snippet should say, beyond its label
        #[arg(long, default_value = "")]
        note: String,
        /// Print the change word by word, `[-removed][+added]`, instead
        #[arg(long)]
        diff: bool,
    },
}

/// An action in the editor's AI menu.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum WriteAction {
    Draft,
    Proofread,
    Clearer,
    Shorter,
    Friendlier,
    Formal,
    Casual,
    Translate,
    Variations,
}

#[derive(Debug, Subcommand)]
pub enum CommandAction {
    /// List the commands, built-in ones first
    List {
        /// Add this library's `type: command` snippets
        #[arg(long)]
        library: Option<PathBuf>,
    },
    /// Run a command on standard input, as if it were the selected text, and
    /// print what would replace it
    Run {
        /// The command's label or id
        name: String,
        #[arg(long)]
        library: Option<PathBuf>,
        /// Print the change word by word, `[-removed][+added]`, instead
        #[arg(long)]
        diff: bool,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum OnOff {
    On,
    Off,
}

#[derive(Debug, Args)]
pub struct KeySource {
    /// Read the key from this environment variable
    #[arg(long, value_name = "VAR", conflicts_with = "key_stdin")]
    key_env: Option<String>,
    /// Read the key from the first line of standard input
    #[arg(long)]
    key_stdin: bool,
}

impl KeySource {
    /// The key asked for, or `None` when neither flag was given.
    fn read(&self) -> Result<Option<Secret>, Failure> {
        let key = if let Some(variable) = &self.key_env {
            std::env::var(variable).map_err(|_| format!("${variable} is not set"))?
        } else if self.key_stdin {
            let stdin = std::io::stdin();
            if stdin.is_terminal() {
                eprintln!("Paste the key and press Return (it will show as you paste):");
            }
            let mut line = String::new();
            stdin.lock().read_line(&mut line)?;
            line
        } else {
            return Ok(None);
        };
        let key = key.trim();
        if key.is_empty() {
            return Err("the key is empty".into());
        }
        Ok(Some(Secret::new(key)))
    }
}

pub fn run(command: AiCommand) -> Result<ExitCode, Failure> {
    let path =
        profiles_path().ok_or("cannot find Aralo's state folder: set HOME or ARALO_STATE")?;
    let settings = AiSettings::open(path)?;
    match command {
        AiCommand::Status => {
            let switches = settings.switches()?;
            println!(
                "AI: {}{}",
                if switches.enabled { "on" } else { "off" },
                if switches.local_only {
                    ", this machine only"
                } else {
                    ""
                }
            );
            let profiles = settings.profiles()?;
            if profiles.is_empty() {
                println!("No profiles yet. See `aralo ai presets`, then `aralo ai add`.");
            }
            for saved in &profiles {
                print_profile(saved);
            }
        }
        AiCommand::On | AiCommand::Off => {
            let enabled = matches!(command, AiCommand::On);
            let switches = settings.switches()?;
            settings.set_switches(AiSwitches {
                enabled,
                ..switches
            })?;
            println!("AI is {}", if enabled { "on" } else { "off" });
        }
        AiCommand::LocalOnly { state } => {
            let local_only = matches!(state, OnOff::On);
            let switches = settings.switches()?;
            settings.set_switches(AiSwitches {
                local_only,
                ..switches
            })?;
            println!(
                "{}",
                if local_only {
                    "Only model servers on this machine will be used"
                } else {
                    "Any saved profile may be used"
                }
            );
        }
        AiCommand::Presets => {
            for preset in PROVIDER_PRESETS {
                println!("{:<11} {:<14} {}", preset.id, preset.name, preset.base_url);
                match preset.key_page {
                    Some(page) => println!("{:<11} key: {page}", ""),
                    None => println!("{:<11} no key needed", ""),
                }
            }
        }
        AiCommand::Add {
            name,
            preset,
            base_url,
            model,
            headers,
            key,
            default,
        } => {
            let preset = match preset.as_deref() {
                Some(id) => Some(provider_preset(id).ok_or_else(|| {
                    format!("there is no preset named {id:?}; see `aralo ai presets`")
                })?),
                None => None,
            };
            let mut draft = match preset {
                Some(preset) => preset.draft(),
                None => ProfileDraft {
                    original_name: None,
                    name: String::new(),
                    adapter: AdapterKind::OpenAiCompat,
                    base_url: String::new(),
                    default_model: String::new(),
                    headers: Vec::new(),
                },
            };
            if let Some(name) = name {
                draft.name = name;
            }
            if draft.name.is_empty() {
                return Err("name the profile, or pick a --preset".into());
            }
            if let Some(base_url) = base_url {
                draft.base_url = base_url;
            } else if preset.is_none() {
                return Err("say where the endpoint is with --base-url, or pick a --preset".into());
            }
            if let Some(model) = model {
                draft.default_model = model;
            }
            draft.headers = parse_headers(&headers)?;
            let key = key.read()?;
            let has_key = key.is_some();
            let saved = settings.save(&draft, key.map_or(KeyChange::Remove, KeyChange::Set))?;
            if default {
                settings.set_default_profile(Some(&saved.profile.name))?;
            }
            println!("Saved {}", saved.profile.name);
            if !has_key {
                if let Some(page) = preset.and_then(|preset| preset.key_page) {
                    println!("It has no key yet. Make one at {page}, then run:");
                    println!("  aralo ai edit {:?} --key-stdin", saved.profile.name);
                }
            }
            println!("Check it with `aralo ai test {:?}`", saved.profile.name);
        }
        AiCommand::Edit {
            name,
            rename,
            base_url,
            model,
            headers,
            clear_headers,
            key,
            remove_key,
        } => {
            let mut draft = settings.profile(&name)?.draft();
            if let Some(rename) = rename {
                draft.name = rename;
            }
            if let Some(base_url) = base_url {
                draft.base_url = base_url;
            }
            if let Some(model) = model {
                draft.default_model = model;
            }
            if clear_headers {
                draft.headers.clear();
            } else if !headers.is_empty() {
                draft.headers = parse_headers(&headers)?;
            }
            let change = match key.read()? {
                Some(key) => KeyChange::Set(key),
                None if remove_key => KeyChange::Remove,
                None => KeyChange::Keep,
            };
            let saved = settings.save(&draft, change)?;
            println!("Saved {}", saved.profile.name);
        }
        AiCommand::Remove { name } => {
            settings.delete(&name)?;
            println!("Removed {name} and its key");
        }
        AiCommand::Default { name } => {
            settings.set_default_profile(Some(&name))?;
            println!("{name} is the default profile");
        }
        AiCommand::Test { name } => {
            let saved = chosen(&settings, name.as_deref())?;
            let report = block_on(settings.test_connection(&saved.draft(), KeyChange::Keep))?;
            println!(
                "{} answered with {} in {} ms: {}",
                saved.profile.name,
                report.model,
                report.first_token.as_millis(),
                report.reply
            );
        }
        AiCommand::Probe { name } => {
            let saved = chosen(&settings, name.as_deref())?;
            let found = block_on(settings.probe(&saved.profile.name))?;
            print_capabilities(&found);
        }
        AiCommand::Models { name } => {
            let saved = chosen(&settings, name.as_deref())?;
            for model in block_on(settings.list_models(&saved.draft(), KeyChange::Keep))? {
                println!("{model}");
            }
        }
        AiCommand::Detect => {
            let servers = block_on(settings.detect_local_servers())?;
            if servers.is_empty() {
                println!("No model server is running on this machine");
                return Ok(ExitCode::FAILURE);
            }
            for server in servers {
                println!(
                    "{:<10} {}  {} {}",
                    server.kind.display_name(),
                    server.base_url,
                    server.models.len(),
                    if server.models.len() == 1 {
                        "model"
                    } else {
                        "models"
                    }
                );
            }
        }
        AiCommand::Command(action) => return run_command(&settings, action),
        AiCommand::Write {
            action,
            language,
            label,
            note,
            diff,
        } => {
            let action = match action {
                WriteAction::Draft => Authoring::Draft { label, note },
                WriteAction::Proofread => Authoring::Proofread,
                WriteAction::Clearer => Authoring::Clearer,
                WriteAction::Shorter => Authoring::Shorter,
                WriteAction::Friendlier => Authoring::Friendlier,
                WriteAction::Formal => Authoring::Formal,
                WriteAction::Casual => Authoring::Casual,
                WriteAction::Translate => Authoring::Translate {
                    language: language.unwrap_or_default(),
                },
                WriteAction::Variations => Authoring::Variations,
            };
            return run_write(&settings, &action, diff);
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn run_command(settings: &AiSettings, action: CommandAction) -> Result<ExitCode, Failure> {
    match action {
        CommandAction::List { library } => {
            for command in commands(library.as_deref())? {
                let origin = if command.builtin {
                    "built-in"
                } else {
                    "library"
                };
                println!("{}  {:<8}  {}", command.id, origin, command.label);
            }
        }
        CommandAction::Run {
            name,
            library,
            diff,
        } => {
            let wanted = name.trim();
            let command = commands(library.as_deref())?
                .into_iter()
                .find(|command| {
                    command.id.to_string().eq_ignore_ascii_case(wanted)
                        || command.label.eq_ignore_ascii_case(wanted)
                })
                .ok_or_else(|| format!("there is no command called \u{201c}{wanted}\u{201d}"))?;
            let mut selection = String::new();
            std::io::stdin().read_to_string(&mut selection)?;
            let run = block_on(async {
                let mut run = settings.run_command(&command, &selection).await?;
                while let Some(piece) = run.next().await {
                    piece.map_err(aralo_core::ai::AiSettingsError::from)?;
                }
                Ok::<_, aralo_core::ai::AiSettingsError>(run)
            })?;
            let sent: Vec<String> = run
                .manifest()
                .iter()
                .map(|entry| format!("{:?} ({} bytes)", entry.kind, entry.bytes.unwrap_or(0)))
                .collect();
            eprintln!(
                "{} with {}; sent {}",
                run.profile(),
                run.model(),
                sent.join(", ")
            );
            let mut out = std::io::stdout().lock();
            if diff {
                for span in run.diff() {
                    match span.change {
                        Change::Same => write!(out, "{}", span.text)?,
                        Change::Removed => write!(out, "[-{}]", span.text)?,
                        Change::Added => write!(out, "[+{}]", span.text)?,
                    }
                }
            } else {
                write!(out, "{}", run.replacement())?;
            }
            out.flush()?;
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Runs an editor action on standard input and prints the answer. Anything
/// about placeholders the answer dropped or added goes to standard error, so
/// standard output is still exactly the text.
fn run_write(settings: &AiSettings, action: &Authoring, diff: bool) -> Result<ExitCode, Failure> {
    let mut text = String::new();
    if action.works_on_text() {
        std::io::stdin().read_to_string(&mut text)?;
    }
    let run = block_on(async {
        let mut run = settings.run_authoring(action, &text).await?;
        while let Some(piece) = run.next().await {
            piece.map_err(aralo_core::ai::AiSettingsError::from)?;
        }
        Ok::<_, aralo_core::ai::AiSettingsError>(run)
    })?;
    eprintln!("{}: {} with {}", action.label(), run.profile(), run.model());
    let versions = run.variations();
    if versions.is_empty() {
        return Err("the model answered with nothing".into());
    }
    let mut out = std::io::stdout().lock();
    for (number, version) in versions.iter().enumerate() {
        if number > 0 {
            writeln!(out, "\n%%%")?;
        }
        if diff && !text.is_empty() {
            for span in aralo_core::diff::diff_words(&text, version) {
                match span.change {
                    Change::Same => write!(out, "{}", span.text)?,
                    Change::Removed => write!(out, "[-{}]", span.text)?,
                    Change::Added => write!(out, "[+{}]", span.text)?,
                }
            }
        } else {
            write!(out, "{version}")?;
        }
        if action.works_on_text() {
            for change in placeholder_changes(&text, version) {
                let what = match change.change {
                    Change::Removed => "dropped",
                    _ => "added",
                };
                eprintln!("aralo: the answer {what} {}", change.placeholder);
            }
        }
    }
    out.flush()?;
    Ok(ExitCode::SUCCESS)
}

/// The built-in commands, and a library's when one is named.
fn commands(library: Option<&Path>) -> Result<Vec<Command>, Failure> {
    Ok(match library {
        Some(library) => Core::open_read_only(library)?.commands(),
        None => builtin_commands(),
    })
}

/// The profile named, or the default one.
fn chosen(settings: &AiSettings, name: Option<&str>) -> Result<SavedProfile, Failure> {
    match name {
        Some(name) => Ok(settings.profile(name)?),
        None => settings
            .default_profile()?
            .ok_or_else(|| "there are no profiles yet; add one with `aralo ai add`".into()),
    }
}

/// The embedding model for `aralo search --model`, loaded through the AI
/// settings: it is a model, so it loads only while AI is on (plan 4.10).
/// While AI is off this says so on standard error and gives `None`, and the
/// search goes by words.
pub fn embedding_model(folder: &Path) -> Result<Option<Model>, Failure> {
    let path =
        profiles_path().ok_or("cannot find Aralo's state folder: set HOME or ARALO_STATE")?;
    let settings = AiSettings::open(path)?;
    if !settings.switches_in_force().enabled {
        eprintln!(
            "aralo: AI is switched off, so the model is not loaded and this searches by words; \
             `aralo ai on` turns it on"
        );
        return Ok(None);
    }
    Ok(Some(settings.load_model(folder)?))
}

/// Who answers an `{{ai}}` block for `aralo expand --ai` and `aralo type --ai`:
/// the AI settings, one block at a time, with the answer read to the end. A
/// refusal or a failure is the reason the block falls back.
pub fn block_answers() -> Result<impl FnMut(&BlockRequest) -> Result<String, String>, Failure> {
    let path =
        profiles_path().ok_or("cannot find Aralo's state folder: set HOME or ARALO_STATE")?;
    let settings = AiSettings::open(path)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    Ok(move |request: &BlockRequest| {
        runtime.block_on(async {
            let run = settings
                .run_block(request)
                .await
                .map_err(|error| error.to_string())?;
            eprintln!(
                "aralo: {} with {} for \u{201c}{}\u{201d}",
                run.profile(),
                run.model(),
                request.block.prompt
            );
            run.finish().await
        })
    })
}

/// Runs one request. The command line does one thing at a time, so a
/// runtime on this thread is enough.
fn block_on<T, E: Into<Failure>>(
    future: impl std::future::Future<Output = Result<T, E>>,
) -> Result<T, Failure> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(future).map_err(Into::into)
}

fn parse_headers(headers: &[String]) -> Result<Vec<(String, String)>, Failure> {
    headers
        .iter()
        .map(|header| {
            header
                .split_once('=')
                .map(|(name, value)| (name.trim().to_owned(), value.trim().to_owned()))
                .ok_or_else(|| format!("write a header as Name=value, not {header:?}").into())
        })
        .collect()
}

fn print_profile(saved: &SavedProfile) {
    let profile = &saved.profile;
    println!(
        "{} {}{}",
        if saved.is_default { "*" } else { " " },
        profile.name,
        if saved.has_key() { "" } else { " (no key)" }
    );
    println!("    {}  {}", profile.base_url, profile.default_model);
    for (name, value) in &profile.headers {
        println!("    {name}: {value}");
    }
    if let Some((found, when)) = &saved.capabilities {
        println!("    probed {when}");
        for (label, check) in rows(found) {
            println!("      {label:<14} {}", check_text(check));
        }
    }
}

fn print_capabilities(found: &Capabilities) {
    for (label, check) in rows(found) {
        println!("{label:<14} {}", check_text(check));
    }
}

fn rows(found: &Capabilities) -> [(&'static str, &Check); 5] {
    [
        ("models list", &found.models_route),
        ("streaming", &found.streaming),
        ("system prompt", &found.system_prompt),
        ("JSON output", &found.json_output),
        ("embeddings", &found.embeddings),
    ]
}

fn check_text(check: &Check) -> String {
    match check {
        Check::Yes => "yes".into(),
        Check::No(reason) => format!("no: {reason}"),
        Check::NotChecked(reason) => format!("not checked: {reason}"),
    }
}
