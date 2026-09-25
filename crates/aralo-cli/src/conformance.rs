//! `aralo conformance`: the conformance suite against a saved profile, and
//! the compatibility table made from its reports (plan 7.4).
//!
//! The profile's key comes from the keychain, as for any feature, or from an
//! environment variable named with `--key-env`, never from an argument. It
//! never reaches a report, which names the endpoint by its host and says only
//! whether a key was sent.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use aralo_core::ai::conformance::{
    bundled_evaluation_set, compatibility_table, planned_endpoints, ConformanceOptions,
    ConformanceReport, Verdict,
};
use aralo_core::ai::{profiles_path, AiSettings, Secret};
use clap::{Args, Subcommand};

use crate::Failure;

/// Where the endpoint list is in a checkout of the repository.
const ENDPOINTS: &str = "conformance/endpoints.toml";

#[derive(Debug, Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct Conformance {
    #[command(subcommand)]
    command: Option<ConformanceCommand>,
    #[command(flatten)]
    run: RunArgs,
}

#[derive(Debug, Args)]
struct RunArgs {
    /// The saved profile to test; the default profile when left out
    #[arg(long)]
    profile: Option<String>,
    /// Run the evaluation set as well: the snippet editor's actions on fixed
    /// texts, scored. It never decides whether an endpoint conforms.
    #[arg(long)]
    evaluation: bool,
    /// Write the JSON report to this file; `-` writes it to standard output
    /// and the summary to standard error
    #[arg(long, value_name = "FILE")]
    report: Option<PathBuf>,
    /// How long one check may take, in seconds
    #[arg(long, default_value_t = 120, value_name = "SECONDS")]
    timeout: u64,
    /// Send the key in this environment variable instead of the profile's,
    /// for this run only: it is saved nowhere. For CI, where the key is a
    /// repository secret and there is no keychain.
    #[arg(long, value_name = "VAR")]
    key_env: Option<String>,
}

#[derive(Debug, Subcommand)]
enum ConformanceCommand {
    /// Make the compatibility table, in Markdown, from reports
    Table {
        /// Report files, or folders of them
        #[arg(required = true)]
        reports: Vec<PathBuf>,
        /// The endpoints the table lists even without a report
        /// [default: conformance/endpoints.toml, if it is there]
        #[arg(long, value_name = "FILE")]
        endpoints: Option<PathBuf>,
        /// Write the table to this file instead of standard output
        #[arg(long, value_name = "FILE")]
        out: Option<PathBuf>,
    },
}

pub fn run(conformance: Conformance) -> Result<ExitCode, Failure> {
    match conformance.command {
        Some(ConformanceCommand::Table {
            reports,
            endpoints,
            out,
        }) => table(&reports, endpoints.as_deref(), out.as_deref()),
        None => suite(&conformance.run),
    }
}

fn suite(args: &RunArgs) -> Result<ExitCode, Failure> {
    let path =
        profiles_path().ok_or("cannot find Aralo's state folder: set HOME or ARALO_STATE")?;
    let settings = AiSettings::open(path)?;
    let name = match &args.profile {
        Some(name) => name.clone(),
        None => {
            settings
                .default_profile()?
                .ok_or("there are no profiles yet; add one with `aralo ai add`")?
                .profile
                .name
        }
    };
    let options = ConformanceOptions {
        evaluation: if args.evaluation {
            Some(bundled_evaluation_set()?)
        } else {
            None
        },
        timeout: Duration::from_secs(args.timeout.max(1)),
        key: match &args.key_env {
            Some(variable) => {
                let key = std::env::var(variable).map_err(|_| format!("${variable} is not set"))?;
                if key.trim().is_empty() {
                    return Err(format!("${variable} is empty").into());
                }
                Some(Secret::new(key.trim()))
            }
            None => None,
        },
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let report = runtime.block_on(settings.conformance(&name, &options))?;

    let to_stdout = args.report.as_deref() == Some(Path::new("-"));
    let summary = summary(&report);
    if to_stdout {
        eprint!("{summary}");
        print!("{}", report.to_json());
    } else {
        print!("{summary}");
        if let Some(path) = &args.report {
            std::fs::write(path, report.to_json())
                .map_err(|error| format!("{}: {error}", path.display()))?;
        }
    }
    Ok(if report.passed {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// What a person reads: every check with what it found, the evaluation, and
/// the verdict, naming the checks that failed.
fn summary(report: &ConformanceReport) -> String {
    let endpoint = &report.endpoint;
    let at = if endpoint.provider == endpoint.host {
        endpoint.host.clone()
    } else {
        format!("{} at {}", endpoint.provider, endpoint.host)
    };
    let mut text = format!(
        "{at}, {}, {}\n",
        endpoint.model,
        if endpoint.key { "with a key" } else { "no key" }
    );
    for result in &report.protocol {
        let required = if result.required {
            ""
        } else {
            " (not required)"
        };
        text.push_str(&format!(
            "  {:<4}  {:<13}  {}{required}\n",
            result.verdict.as_str(),
            result.check,
            result.detail
        ));
    }
    if let Some(evaluation) = &report.evaluation {
        text.push_str(&format!(
            "Evaluation: {} of {} as expected\n",
            evaluation.passed,
            evaluation.cases.len()
        ));
        for case in &evaluation.cases {
            text.push_str(&format!(
                "  {:<4}  {}: {}\n",
                case.verdict.as_str(),
                case.case,
                case.detail
            ));
            if case.verdict != Verdict::Pass && !case.answer.is_empty() {
                text.push_str(&format!("        the answer: {}\n", case.answer));
            }
        }
    }
    let failed: Vec<&str> = report
        .failures()
        .map(|result| result.check.as_str())
        .collect();
    if failed.is_empty() {
        text.push_str("It conforms.\n");
    } else {
        text.push_str(&format!(
            "It does not conform: {} failed.\n",
            failed.join(", ")
        ));
    }
    text
}

fn table(
    reports: &[PathBuf],
    endpoints: Option<&Path>,
    out: Option<&Path>,
) -> Result<ExitCode, Failure> {
    let mut files = Vec::new();
    for path in reports {
        if path.is_dir() {
            let mut found: Vec<PathBuf> = std::fs::read_dir(path)
                .map_err(|error| format!("{}: {error}", path.display()))?
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|file| {
                    file.extension()
                        .is_some_and(|extension| extension == "json")
                })
                .collect();
            found.sort();
            files.extend(found);
        } else {
            files.push(path.clone());
        }
    }
    let mut read = Vec::with_capacity(files.len());
    for file in &files {
        let text = std::fs::read_to_string(file)
            .map_err(|error| format!("{}: {error}", file.display()))?;
        read.push(
            ConformanceReport::from_json(&text)
                .map_err(|problem| format!("{}: {problem}", file.display()))?,
        );
    }
    let planned = match endpoints {
        Some(path) => Some(path.to_path_buf()),
        None => Some(PathBuf::from(ENDPOINTS)).filter(|path| path.exists()),
    };
    let planned = match planned {
        Some(path) => {
            let text = std::fs::read_to_string(&path)
                .map_err(|error| format!("{}: {error}", path.display()))?;
            planned_endpoints(&text).map_err(|problem| format!("{}: {problem}", path.display()))?
        }
        None => Vec::new(),
    };
    let page = compatibility_table(&read, &planned);
    match out {
        Some(path) => {
            std::fs::write(path, page).map_err(|error| format!("{}: {error}", path.display()))?
        }
        None => print!("{page}"),
    }
    Ok(ExitCode::SUCCESS)
}
