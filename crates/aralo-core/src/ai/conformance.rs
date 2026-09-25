//! The conformance suite (plan 7.4, task 4.9), which `aralo conformance`
//! runs against a saved profile.
//!
//! It has two layers. The protocol checks are what Aralo needs from an
//! endpoint to work at all, and an endpoint conforms when no required one
//! fails:
//!
//! | Check | Passes when |
//! | --- | --- |
//! | `models` | `GET /models` lists at least one model |
//! | `stream` | an answer arrives in more than one piece and ends with a stop reason |
//! | `system_prompt` | the model does what the system prompt asks, which the user message never mentions |
//! | `cancel` | a stream dropped after its first piece leaves the endpoint answering the next request |
//! | `unknown_model` | a model the endpoint does not have is refused with a 4xx Aralo can read |
//! | `wrong_key` | a wrong key is refused with a 4xx Aralo can read |
//! | `usage` | the stream reports token counts, which metering needs (not required) |
//! | `json_output` | JSON mode answers with a JSON object (not required) |
//!
//! An error check is skipped, not failed, when the endpoint answers anyway: a
//! local server that serves the one model it has loaded, whatever the name,
//! gives Aralo no error to read.
//!
//! The evaluation set runs the snippet editor's own actions on fixed texts
//! ([`EVALUATION_SET`], from `conformance/evaluation.toml`) and checks the
//! answers: a grammar fix, tone shifts, and a date placeholder kept and
//! written. It says how well a model does Aralo's work and never decides
//! conformance.
//!
//! Every request goes through the gateway under [`Feature::Conformance`], so
//! the policy check, the network guard and metering are those of any feature.
//! A report names the endpoint by its host alone, since a path or a query can
//! hold a key, and says whether a key was sent, never what it was.
//! [`compatibility_table`] turns reports into `docs/compatibility.md`.

use std::collections::BTreeMap;
use std::future::Future;
use std::net::IpAddr;
use std::time::{Duration, Instant};

use aralo_ai::{AiError, AiRequest, AiStream, Delta, Feature, Host, Profile, Refusal, Secret};
use aralo_ai::{StopReason, Usage};
use aralo_providers::LocalServerKind;
use serde::{Deserialize, Serialize};

use super::authoring::{placeholder_changes, Authoring};
use super::presets::PROVIDER_PRESETS;
use super::{looks_like_key, AiSettings, AiSettingsError};
use crate::diff::Change;

/// The version of the report's shape. A table is made only from reports of
/// this version.
pub const REPORT_FORMAT: u32 = 1;

/// The evaluation set, `conformance/evaluation.toml`.
pub const EVALUATION_SET: &str = include_str!("../../../../conformance/evaluation.toml");

/// How long one check may take unless the caller says otherwise. The first
/// request to a local server may wait while it loads the model.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

/// Room for a short answer from a model that reasons before it answers.
const MAX_TOKENS: u32 = 1024;
/// Room for an answer long enough to be cancelled halfway.
const CANCEL_MAX_TOKENS: u32 = 2048;
/// A model name no endpoint serves.
const NO_SUCH_MODEL: &str = "aralo-conformance-no-such-model";
/// A key no endpoint accepts.
const WRONG_KEY: &str = "aralo-conformance-wrong-key";
/// A detail or an answer in a report is cut to this many characters.
const MAX_DETAIL: usize = 300;
/// What the model wrote, quoted in a failure, is cut to this many.
const MAX_QUOTE: usize = 80;

const COUNT_TO_TEN: &str =
    "Write the numbers from one to ten in words, separated by spaces, and nothing else.";
const COUNT_LONG: &str =
    "Write the numbers from one to two hundred in words, one on each line, and nothing else.";
const REPLY_OK: &str = "Reply with the single word: ok";
const PINEAPPLE_SYSTEM: &str =
    "Whatever the user writes, reply with the single word PINEAPPLE and nothing else.";
const PINEAPPLE_USER: &str = "Hello.";

/// One protocol check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProtocolCheck {
    Models,
    Stream,
    SystemPrompt,
    Cancel,
    UnknownModel,
    WrongKey,
    Usage,
    JsonOutput,
}

impl ProtocolCheck {
    /// In the order they run and the table shows them.
    pub const ALL: [Self; 8] = [
        Self::Models,
        Self::Stream,
        Self::SystemPrompt,
        Self::Cancel,
        Self::UnknownModel,
        Self::WrongKey,
        Self::Usage,
        Self::JsonOutput,
    ];

    /// What a report calls it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Models => "models",
            Self::Stream => "stream",
            Self::SystemPrompt => "system_prompt",
            Self::Cancel => "cancel",
            Self::UnknownModel => "unknown_model",
            Self::WrongKey => "wrong_key",
            Self::Usage => "usage",
            Self::JsonOutput => "json_output",
        }
    }

    /// What the table calls it.
    pub fn label(self) -> &'static str {
        match self {
            Self::Models => "Models",
            Self::Stream => "Stream",
            Self::SystemPrompt => "System prompt",
            Self::Cancel => "Cancel",
            Self::UnknownModel => "Unknown model",
            Self::WrongKey => "Wrong key",
            Self::Usage => "Usage",
            Self::JsonOutput => "JSON",
        }
    }

    /// Whether an endpoint conforms only if this does not fail. Aralo works
    /// without usage, and no feature needs JSON mode yet.
    pub fn required(self) -> bool {
        !matches!(self, Self::Usage | Self::JsonOutput)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Pass,
    Fail,
    /// There was nothing to check, and the detail says why.
    Skip,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
            Self::Skip => "skip",
        }
    }
}

/// What one protocol check found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckResult {
    /// [`ProtocolCheck::name`].
    pub check: String,
    pub required: bool,
    pub verdict: Verdict,
    /// What happened, in a sentence: the finding for a pass, the reason for
    /// a failure or a skip.
    pub detail: String,
    /// How long the check took, in milliseconds.
    pub ms: u64,
}

/// The endpoint a report is about. There is no key in it, and no address
/// beyond the host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestedEndpoint {
    /// The provider or server, from its address: a preset's name, a local
    /// server's on its default port, or the host.
    pub provider: String,
    pub adapter: String,
    /// The host and port.
    pub host: String,
    pub model: String,
    /// On this machine.
    pub local: bool,
    /// Whether a key was sent.
    pub key: bool,
}

/// What one case of the evaluation set found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaseResult {
    pub case: String,
    pub verdict: Verdict,
    /// What was wrong with the answer, or that nothing was.
    pub detail: String,
    /// The start of the answer.
    pub answer: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluationResult {
    /// How many cases passed.
    pub passed: usize,
    pub cases: Vec<CaseResult>,
}

/// What `aralo conformance` writes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConformanceReport {
    /// [`REPORT_FORMAT`].
    pub format: u32,
    /// The version of Aralo that ran it.
    pub aralo: String,
    /// When it ran, `YYYY-MM-DD`, in UTC.
    pub date: String,
    pub endpoint: TestedEndpoint,
    /// No required check failed.
    pub passed: bool,
    /// How long the stream check waited for the first piece of its answer.
    pub first_token_ms: Option<u64>,
    pub protocol: Vec<CheckResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluation: Option<EvaluationResult>,
}

impl ConformanceReport {
    /// The report as it is written to a file.
    pub fn to_json(&self) -> String {
        let mut json = serde_json::to_string_pretty(self)
            .unwrap_or_else(|error| format!("{{\"error\": \"{error}\"}}"));
        json.push('\n');
        json
    }

    pub fn from_json(text: &str) -> Result<Self, String> {
        let report: Self = serde_json::from_str(text).map_err(|error| error.to_string())?;
        if report.format != REPORT_FORMAT {
            return Err(format!(
                "the report is format {}, and this version of Aralo reads format {REPORT_FORMAT}",
                report.format
            ));
        }
        Ok(report)
    }

    pub fn check(&self, check: ProtocolCheck) -> Option<&CheckResult> {
        self.protocol
            .iter()
            .find(|result| result.check == check.name())
    }

    /// The required checks that failed, which is why an endpoint does not
    /// conform.
    pub fn failures(&self) -> impl Iterator<Item = &CheckResult> {
        self.protocol
            .iter()
            .filter(|result| result.required && result.verdict == Verdict::Fail)
    }
}

/// How a run goes.
#[derive(Debug)]
pub struct ConformanceOptions {
    /// The evaluation set to run after the protocol checks, if any.
    pub evaluation: Option<Vec<EvaluationCase>>,
    /// How long one check, or one case, may take.
    pub timeout: Duration,
    /// A key to send instead of the profile's own, for a run the keychain
    /// has no part in, such as CI's with a key from a repository secret. It
    /// is used for the run and saved nowhere.
    pub key: Option<Secret>,
}

impl Default for ConformanceOptions {
    fn default() -> Self {
        Self {
            evaluation: None,
            timeout: DEFAULT_TIMEOUT,
            key: None,
        }
    }
}

impl AiSettings {
    /// Runs the conformance suite against a saved profile, with its key or
    /// the one the options give.
    ///
    /// It is refused while AI is off, and for a remote profile in local-only
    /// mode, before anything is sent. Anything the endpoint does is a finding
    /// in the report.
    pub async fn conformance(
        &self,
        name: &str,
        options: &ConformanceOptions,
    ) -> Result<ConformanceReport, AiSettingsError> {
        self.ensure_on()?;
        let profile = self.profile(name)?.profile;
        self.gateway().policy().check(&profile)?;
        let key = match &options.key {
            Some(key) => Some(key.duplicate()),
            None => self.key_for(&profile)?,
        };
        let runner = Runner {
            settings: self,
            profile: &profile,
            key: key.as_ref(),
            timeout: options.timeout,
        };

        let models = runner.models().await?;
        let (stream, answer) = runner.stream().await?;
        let usage = usage_check(answer.as_ref());
        let system_prompt = runner.system_prompt().await?;
        let cancel = runner.cancel().await?;
        let unknown_model = runner.unknown_model().await?;
        let wrong_key = runner.wrong_key().await?;
        let json_output = runner.json_output().await?;
        let protocol = vec![
            models,
            stream,
            system_prompt,
            cancel,
            unknown_model,
            wrong_key,
            usage,
            json_output,
        ];
        let evaluation = match &options.evaluation {
            Some(cases) => Some(runner.evaluate(cases).await?),
            None => None,
        };

        let (provider, host, local) = describe(&profile);
        Ok(ConformanceReport {
            format: REPORT_FORMAT,
            aralo: env!("CARGO_PKG_VERSION").to_owned(),
            date: (self.clock)().chars().take(10).collect(),
            endpoint: TestedEndpoint {
                provider,
                adapter: profile.adapter.as_str().to_owned(),
                host,
                model: profile.default_model.clone(),
                local,
                key: key.is_some(),
            },
            passed: protocol
                .iter()
                .all(|result| !(result.required && result.verdict == Verdict::Fail)),
            first_token_ms: answer
                .as_ref()
                .and_then(|answer| answer.first_token)
                .map(millis),
            protocol,
            evaluation,
        })
    }
}

/// The profile under test and what it runs with.
struct Runner<'a> {
    settings: &'a AiSettings,
    profile: &'a Profile,
    key: Option<&'a Secret>,
    timeout: Duration,
}

/// A whole answer, and how it came.
#[derive(Debug, Default)]
struct Answer {
    pieces: usize,
    text: String,
    first_token: Option<Duration>,
    usage: Option<Usage>,
    stop: Option<StopReason>,
}

impl Runner<'_> {
    fn request(&self, system: &str, instruction: &str, max_tokens: u32) -> AiRequest {
        AiRequest {
            feature: Feature::Conformance,
            profile: self.profile.clone(),
            model: None,
            system: system.to_owned(),
            instruction: instruction.to_owned(),
            declared: Vec::new(),
            max_tokens: Some(max_tokens),
            // Left to the endpoint, as the probe leaves it: some reasoning
            // models refuse any other value.
            temperature: None,
            json_output: false,
        }
    }

    /// Sends a request with `key`, which the error checks replace.
    async fn open(&self, request: AiRequest, key: Option<&Secret>) -> Result<AiStream, AiError> {
        let gateway = self.settings.gateway();
        let prepared = gateway.prepare(request, &mut ())?;
        gateway.send(prepared, key.map(Secret::duplicate)).await
    }

    /// Sends a request with `key` and reads the answer to its end.
    async fn ask_with(&self, request: AiRequest, key: Option<&Secret>) -> Result<Answer, AiError> {
        self.within(async {
            let started = Instant::now();
            let stream = self.open(request, key).await?;
            read(stream, started).await
        })
        .await
    }

    async fn ask(&self, request: AiRequest) -> Result<Answer, AiError> {
        self.ask_with(request, self.key).await
    }

    async fn within<T>(
        &self,
        work: impl Future<Output = Result<T, AiError>>,
    ) -> Result<T, AiError> {
        tokio::time::timeout(self.timeout, work)
            .await
            .unwrap_or_else(|_| Err(self.timed_out()))
    }

    fn timed_out(&self) -> AiError {
        AiError::Network(format!(
            "nothing came back within {} s",
            self.timeout.as_secs()
        ))
    }

    async fn models(&self) -> Result<CheckResult, Refusal> {
        let started = Instant::now();
        let listed = self
            .within(
                self.settings
                    .gateway()
                    .list_models(self.profile, self.key.map(Secret::duplicate)),
            )
            .await;
        let (verdict, detail) = match listed {
            Err(AiError::Refused(refusal)) => return Err(refusal),
            Err(error) => (Verdict::Fail, error.to_string()),
            Ok(models) if models.is_empty() => (
                Verdict::Fail,
                "the models route answered with no models".to_owned(),
            ),
            Ok(models) => {
                let own = if models.contains(&self.profile.default_model) {
                    "the profile's model is one"
                } else {
                    "the profile's model is not one of them"
                };
                (
                    Verdict::Pass,
                    format!("{}; {own}", count(models.len(), "model")),
                )
            }
        };
        Ok(result(ProtocolCheck::Models, verdict, detail, started))
    }

    /// The stream check, and the answer it read, whose first token and usage
    /// the report and the usage check need.
    async fn stream(&self) -> Result<(CheckResult, Option<Answer>), Refusal> {
        let started = Instant::now();
        let answer = match self.ask(self.request("", COUNT_TO_TEN, MAX_TOKENS)).await {
            Err(AiError::Refused(refusal)) => return Err(refusal),
            Err(error) => {
                let failed = result(
                    ProtocolCheck::Stream,
                    Verdict::Fail,
                    error.to_string(),
                    started,
                );
                return Ok((failed, None));
            }
            Ok(answer) => answer,
        };
        let (verdict, detail) = match &answer.stop {
            None => (
                Verdict::Fail,
                "the stream ended without saying why the model stopped".to_owned(),
            ),
            Some(stop) if answer.text.trim().is_empty() => (
                Verdict::Fail,
                format!(
                    "the model wrote nothing, and was stopped by {}",
                    stopped_by(stop)
                ),
            ),
            Some(_) if answer.pieces < 2 => (
                Verdict::Fail,
                "the answer came back whole, not in pieces".to_owned(),
            ),
            Some(stop) => (
                Verdict::Pass,
                format!(
                    "{} pieces, the first after {} ms; stopped by {}",
                    answer.pieces,
                    answer.first_token.map(millis).unwrap_or_default(),
                    stopped_by(stop)
                ),
            ),
        };
        Ok((
            result(ProtocolCheck::Stream, verdict, detail, started),
            Some(answer),
        ))
    }

    async fn system_prompt(&self) -> Result<CheckResult, Refusal> {
        let started = Instant::now();
        let asked = self
            .ask(self.request(PINEAPPLE_SYSTEM, PINEAPPLE_USER, MAX_TOKENS))
            .await;
        let (verdict, detail) = match asked {
            Err(AiError::Refused(refusal)) => return Err(refusal),
            Err(error) => (Verdict::Fail, error.to_string()),
            Ok(answer) if has(&normalised(&answer.text), "pineapple") => (
                Verdict::Pass,
                "the model answered as the system prompt asked".to_owned(),
            ),
            Ok(answer) => (
                Verdict::Fail,
                format!(
                    "the model did not do what the system prompt asked: it wrote \u{201c}{}\u{201d}",
                    clip(answer.text.trim(), MAX_QUOTE)
                ),
            ),
        };
        Ok(result(
            ProtocolCheck::SystemPrompt,
            verdict,
            detail,
            started,
        ))
    }

    /// Drops a long answer at its first piece, then sends the next request.
    /// A stream dropped here closes its connection (the gateway's contract,
    /// which `aralo-providers/tests/network.rs` holds it to); what an
    /// endpoint can show from outside is that it is ready for more.
    async fn cancel(&self) -> Result<CheckResult, Refusal> {
        let started = Instant::now();
        let first = self
            .within(async {
                let mut stream = self
                    .open(self.request("", COUNT_LONG, CANCEL_MAX_TOKENS), self.key)
                    .await?;
                loop {
                    match stream.next().await {
                        Some(Ok(Delta::Text(_))) => {
                            let at = started.elapsed();
                            drop(stream);
                            return Ok(Some(at));
                        }
                        Some(Ok(_)) => {}
                        Some(Err(error)) => return Err(error),
                        None => return Ok(None),
                    }
                }
            })
            .await;
        let cancelled_at = match first {
            Err(AiError::Refused(refusal)) => return Err(refusal),
            Err(error) => {
                return Ok(result(
                    ProtocolCheck::Cancel,
                    Verdict::Fail,
                    error.to_string(),
                    started,
                ))
            }
            Ok(None) => {
                return Ok(result(
                    ProtocolCheck::Cancel,
                    Verdict::Fail,
                    "the answer ended before a piece of it came to cancel",
                    started,
                ))
            }
            Ok(Some(at)) => millis(at),
        };
        let (verdict, detail) = match self.ask(self.request("", REPLY_OK, MAX_TOKENS)).await {
            Err(AiError::Refused(refusal)) => return Err(refusal),
            Err(error) => (
                Verdict::Fail,
                format!(
                    "cancelled at the first piece, {cancelled_at} ms in, and then the next \
                     request failed: {error}"
                ),
            ),
            Ok(answer) if answer.text.trim().is_empty() => (
                Verdict::Fail,
                format!(
                    "cancelled at the first piece, {cancelled_at} ms in, and then the next \
                     request was answered with nothing"
                ),
            ),
            Ok(answer) => (
                Verdict::Pass,
                format!(
                    "cancelled at the first piece, {cancelled_at} ms in; the next request's \
                     first piece came {} ms after it was sent",
                    answer.first_token.map(millis).unwrap_or_default()
                ),
            ),
        };
        Ok(result(ProtocolCheck::Cancel, verdict, detail, started))
    }

    async fn unknown_model(&self) -> Result<CheckResult, Refusal> {
        let started = Instant::now();
        let mut request = self.request("", REPLY_OK, MAX_TOKENS);
        request.model = Some(NO_SUCH_MODEL.to_owned());
        let (verdict, detail) = refused_as_asked(
            self.ask(request).await,
            "the endpoint answered anyway: it serves the model it has, whatever the name",
        )?;
        Ok(result(
            ProtocolCheck::UnknownModel,
            verdict,
            detail,
            started,
        ))
    }

    async fn wrong_key(&self) -> Result<CheckResult, Refusal> {
        let started = Instant::now();
        if self.key.is_none() {
            return Ok(result(
                ProtocolCheck::WrongKey,
                Verdict::Skip,
                "the profile sends no key",
                started,
            ));
        }
        let wrong = Secret::new(WRONG_KEY);
        let (verdict, detail) = refused_as_asked(
            self.ask_with(self.request("", REPLY_OK, MAX_TOKENS), Some(&wrong))
                .await,
            "the endpoint answered with a wrong key: it does not check keys",
        )?;
        Ok(result(ProtocolCheck::WrongKey, verdict, detail, started))
    }

    async fn json_output(&self) -> Result<CheckResult, Refusal> {
        let started = Instant::now();
        let mut request = self.request(
            "Reply with a JSON object and nothing else.",
            "Return an object with one key, \"ok\", set to true.",
            MAX_TOKENS,
        );
        request.json_output = true;
        let (verdict, detail) = match self.ask(request).await {
            Err(AiError::Refused(refusal)) => return Err(refusal),
            Err(error) => (Verdict::Fail, error.to_string()),
            Ok(answer) => {
                let object = serde_json::from_str::<serde_json::Value>(answer.text.trim())
                    .is_ok_and(|value| value.is_object());
                if object {
                    (Verdict::Pass, "the answer was a JSON object".to_owned())
                } else {
                    (
                        Verdict::Fail,
                        format!(
                            "the answer in JSON mode was not a JSON object: \u{201c}{}\u{201d}",
                            clip(answer.text.trim(), MAX_QUOTE)
                        ),
                    )
                }
            }
        };
        Ok(result(ProtocolCheck::JsonOutput, verdict, detail, started))
    }

    async fn evaluate(&self, cases: &[EvaluationCase]) -> Result<EvaluationResult, Refusal> {
        let mut results = Vec::with_capacity(cases.len());
        for case in cases {
            results.push(self.evaluate_case(case).await?);
        }
        Ok(EvaluationResult {
            passed: results
                .iter()
                .filter(|result| result.verdict == Verdict::Pass)
                .count(),
            cases: results,
        })
    }

    /// Runs one case through the editor's own action, on the profile under
    /// test, and scores what would go into the editor.
    async fn evaluate_case(&self, case: &EvaluationCase) -> Result<CaseResult, Refusal> {
        let failed = |detail: String| CaseResult {
            case: case.name.clone(),
            verdict: Verdict::Fail,
            detail: scrub(&detail),
            answer: String::new(),
        };
        let action = match case.authoring() {
            Ok(action) => action,
            Err(problem) => return Ok(failed(problem)),
        };
        let original = if action.works_on_text() {
            case.text.clone()
        } else {
            String::new()
        };
        let ran = tokio::time::timeout(self.timeout, async {
            let mut run = self
                .settings
                .start_authoring(
                    &action,
                    original,
                    self.profile.clone(),
                    self.key.map(Secret::duplicate),
                    Feature::Conformance,
                )
                .await?;
            while let Some(piece) = run.next().await {
                piece?;
            }
            Ok::<_, AiSettingsError>(run)
        })
        .await;
        let run = match ran {
            Err(_) => return Ok(failed(self.timed_out().to_string())),
            Ok(Err(AiSettingsError::Ai(AiError::Refused(refusal)))) => return Err(refusal),
            Ok(Err(error)) => return Ok(failed(error.to_string())),
            Ok(Ok(run)) => run,
        };
        let answer = run.answer();
        let problems = case.problems(&answer);
        Ok(CaseResult {
            case: case.name.clone(),
            verdict: if problems.is_empty() {
                Verdict::Pass
            } else {
                Verdict::Fail
            },
            detail: if problems.is_empty() {
                "as expected".to_owned()
            } else {
                scrub(&problems.join("; "))
            },
            answer: scrub(answer.trim()),
        })
    }
}

async fn read(mut stream: AiStream, started: Instant) -> Result<Answer, AiError> {
    let mut answer = Answer::default();
    while let Some(delta) = stream.next().await {
        match delta? {
            Delta::Text(piece) => {
                answer.first_token.get_or_insert_with(|| started.elapsed());
                answer.pieces += 1;
                answer.text.push_str(&piece);
            }
            Delta::Usage(usage) => answer.usage = Some(usage),
            Delta::Done(stop) => answer.stop = Some(stop),
        }
    }
    Ok(answer)
}

/// What an endpoint did with a request it should refuse: a 4xx passes, since
/// Aralo read it; an answer means there was nothing to read.
fn refused_as_asked(
    outcome: Result<Answer, AiError>,
    answered: &str,
) -> Result<(Verdict, String), Refusal> {
    Ok(match outcome {
        Err(AiError::Refused(refusal)) => return Err(refusal),
        Err(AiError::Status {
            status, message, ..
        }) if (400..500).contains(&status) && status != 429 => {
            (Verdict::Pass, format!("{status}: {message}"))
        }
        Err(AiError::Status {
            status, message, ..
        }) => (
            Verdict::Fail,
            format!("the endpoint answered {status}: {message}; a request in error is a 4xx"),
        ),
        Err(error) => (Verdict::Fail, error.to_string()),
        Ok(_) => (Verdict::Skip, answered.to_owned()),
    })
}

fn usage_check(answer: Option<&Answer>) -> CheckResult {
    let (verdict, detail) = match answer {
        None => (Verdict::Skip, "the stream check failed".to_owned()),
        Some(Answer {
            usage: Some(usage), ..
        }) if usage.total() > 0 => {
            let cached = if usage.cached_input_tokens > 0 {
                format!(", {} of them cached", usage.cached_input_tokens)
            } else {
                String::new()
            };
            (
                Verdict::Pass,
                format!(
                    "{} in{cached}, {} out",
                    count(usage.input_tokens as usize, "token"),
                    usage.output_tokens
                ),
            )
        }
        Some(_) => (
            Verdict::Fail,
            "the stream reported no token counts, so metering cannot count them".to_owned(),
        ),
    };
    CheckResult {
        check: ProtocolCheck::Usage.name().to_owned(),
        required: ProtocolCheck::Usage.required(),
        verdict,
        detail,
        ms: 0,
    }
}

fn result(
    check: ProtocolCheck,
    verdict: Verdict,
    detail: impl AsRef<str>,
    started: Instant,
) -> CheckResult {
    CheckResult {
        check: check.name().to_owned(),
        required: check.required(),
        verdict,
        detail: scrub(detail.as_ref()),
        ms: millis(started.elapsed()),
    }
}

fn stopped_by(stop: &StopReason) -> String {
    match stop {
        StopReason::End => "the model".to_owned(),
        StopReason::Length => "the token limit".to_owned(),
        StopReason::Other(reason) => format!("\u{201c}{reason}\u{201d}"),
    }
}

/// The provider, the host and port, and whether it is this machine.
fn describe(profile: &Profile) -> (String, String, bool) {
    let Ok(endpoint) = aralo_ai::guard::parse_endpoint(&profile.base_url) else {
        return ("unknown".to_owned(), String::new(), false);
    };
    let host = match &endpoint.host {
        Host::Ip(IpAddr::V6(ip)) => format!("[{ip}]:{}", endpoint.port),
        host => format!("{host}:{}", endpoint.port),
    };
    let local = endpoint.host.is_loopback();
    let provider = if local {
        LocalServerKind::ALL
            .into_iter()
            .find(|kind| kind.default_port() == endpoint.port)
            .map(LocalServerKind::display_name)
    } else {
        PROVIDER_PRESETS
            .iter()
            .find(|preset| {
                aralo_ai::guard::parse_endpoint(preset.base_url)
                    .is_ok_and(|known| known.host == endpoint.host)
            })
            .map(|preset| preset.name)
    };
    (
        provider.map_or_else(|| host.clone(), str::to_owned),
        host,
        local,
    )
}

/// A detail or an answer as a report holds it: short, and with anything that
/// looks like a key taken out.
fn scrub(text: &str) -> String {
    let words: Vec<&str> = text
        .split(' ')
        .map(|word| {
            if looks_like_key(word) {
                "[redacted]"
            } else {
                word
            }
        })
        .collect();
    clip(&words.join(" "), MAX_DETAIL)
}

fn clip(text: &str, limit: usize) -> String {
    match text.char_indices().nth(limit) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_owned(),
    }
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn count(number: usize, what: &str) -> String {
    if number == 1 {
        format!("1 {what}")
    } else {
        format!("{number} {what}s")
    }
}

// The evaluation set.

/// One case of the evaluation set, as `conformance/evaluation.toml` writes it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationCase {
    pub name: String,
    /// `draft`, `proofread`, `clearer`, `shorter`, `friendlier`, `formal`,
    /// `casual` or `translate`.
    pub action: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub contains: Vec<String>,
    #[serde(default)]
    pub lacks: Vec<String>,
    #[serde(default)]
    pub any_of: Vec<String>,
    #[serde(default)]
    pub keeps_placeholders: bool,
    pub at_most: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvaluationFile {
    #[serde(rename = "case", default)]
    cases: Vec<EvaluationCase>,
}

/// Reads an evaluation set, and checks that every case names an action and
/// expects something.
pub fn evaluation_set(text: &str) -> Result<Vec<EvaluationCase>, String> {
    let file: EvaluationFile = toml::from_str(text).map_err(|error| error.to_string())?;
    for case in &file.cases {
        let action = case.authoring()?;
        let expects = !case.contains.is_empty()
            || !case.lacks.is_empty()
            || !case.any_of.is_empty()
            || case.keeps_placeholders
            || case.at_most.is_some();
        if !expects {
            return Err(format!("{}: the case expects nothing", case.name));
        }
        if action.works_on_text() && case.text.trim().is_empty() {
            return Err(format!("{}: the case has no text to work on", case.name));
        }
    }
    Ok(file.cases)
}

/// The evaluation set compiled in.
pub fn bundled_evaluation_set() -> Result<Vec<EvaluationCase>, String> {
    evaluation_set(EVALUATION_SET)
}

impl EvaluationCase {
    /// The editor action the case runs.
    pub fn authoring(&self) -> Result<Authoring, String> {
        Ok(match self.action.trim() {
            "draft" => Authoring::Draft {
                label: self.label.clone(),
                note: self.note.clone(),
            },
            "proofread" => Authoring::Proofread,
            "clearer" => Authoring::Clearer,
            "shorter" => Authoring::Shorter,
            "friendlier" => Authoring::Friendlier,
            "formal" => Authoring::Formal,
            "casual" => Authoring::Casual,
            "translate" => Authoring::Translate {
                language: self.language.clone(),
            },
            other => {
                return Err(format!(
                    "{}: \u{201c}{other}\u{201d} is not an editor action",
                    self.name
                ))
            }
        })
    }

    /// What is wrong with an answer, a phrase each. Empty when it passes.
    pub fn problems(&self, answer: &str) -> Vec<String> {
        let found = normalised(answer);
        if found.trim().is_empty() {
            return vec!["the model wrote nothing".to_owned()];
        }
        let mut problems = Vec::new();
        for wanted in &self.contains {
            if !has(&found, wanted) {
                problems.push(format!("does not have \u{201c}{wanted}\u{201d}"));
            }
        }
        for unwanted in &self.lacks {
            if has(&found, unwanted) {
                problems.push(format!("still has \u{201c}{unwanted}\u{201d}"));
            }
        }
        if !self.any_of.is_empty() && !self.any_of.iter().any(|wanted| has(&found, wanted)) {
            let quoted: Vec<String> = self
                .any_of
                .iter()
                .map(|wanted| format!("\u{201c}{wanted}\u{201d}"))
                .collect();
            problems.push(format!("has none of {}", quoted.join(", ")));
        }
        if self.keeps_placeholders {
            for change in placeholder_changes(&self.text, answer) {
                let what = if change.change == Change::Removed {
                    "dropped"
                } else {
                    "added"
                };
                problems.push(format!("{what} {}", change.placeholder));
            }
        }
        if let Some(limit) = self.at_most {
            let ratio = answer.trim().chars().count() as f64
                / self.text.trim().chars().count().max(1) as f64;
            if ratio > limit {
                problems.push(format!(
                    "is {:.0}% of the text's length, over {:.0}%",
                    ratio * 100.0,
                    limit * 100.0
                ));
            }
        }
        problems
    }
}

/// Text as the evaluation compares it: lower case, curly quotes straight.
fn normalised(text: &str) -> String {
    text.to_lowercase()
        .replace(['\u{2018}', '\u{2019}'], "'")
        .replace(['\u{201c}', '\u{201d}'], "\"")
}

/// Whether `text`, normalised, holds `needle` as whole words: an end of the
/// needle that is a letter or a digit may not run on into another.
fn has(text: &str, needle: &str) -> bool {
    let needle = normalised(needle);
    let Some(first) = needle.chars().next() else {
        return true;
    };
    let starts_word = first.is_alphanumeric();
    let ends_word = needle
        .chars()
        .next_back()
        .is_some_and(char::is_alphanumeric);
    text.match_indices(&needle).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + needle.len()..].chars().next();
        !(starts_word && before.is_some_and(char::is_alphanumeric))
            && !(ends_word && after.is_some_and(char::is_alphanumeric))
    })
}

// The compatibility table.

/// An endpoint the suite is meant to run against, as
/// `conformance/endpoints.toml` lists it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlannedEndpoint {
    pub name: String,
    pub base_url: String,
    pub model: String,
    /// The environment variable, and the repository secret, holding a key.
    #[serde(default)]
    pub key_env: Option<String>,
    /// When CI runs it.
    pub runs: String,
    /// The recorded stream the mock server replays.
    #[serde(default)]
    pub transcript: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EndpointsFile {
    #[serde(rename = "endpoint", default)]
    endpoints: Vec<PlannedEndpoint>,
}

pub fn planned_endpoints(text: &str) -> Result<Vec<PlannedEndpoint>, String> {
    let file: EndpointsFile = toml::from_str(text).map_err(|error| error.to_string())?;
    Ok(file.endpoints)
}

/// The compatibility table, `docs/compatibility.md`: a row for the latest
/// report on each endpoint and model, what did not pass and why, and the
/// planned endpoints with no report yet.
pub fn compatibility_table(reports: &[ConformanceReport], planned: &[PlannedEndpoint]) -> String {
    // The latest report for each endpoint and model. Reports of one day are
    // kept in the order given.
    let mut latest: BTreeMap<(String, String), &ConformanceReport> = BTreeMap::new();
    for report in reports {
        let key = (
            report.endpoint.provider.clone(),
            report.endpoint.model.clone(),
        );
        if latest.get(&key).is_none_or(|kept| kept.date <= report.date) {
            latest.insert(key, report);
        }
    }
    let mut rows: Vec<&ConformanceReport> = latest.into_values().collect();
    let place = |report: &ConformanceReport| {
        planned
            .iter()
            .position(|endpoint| endpoint.name == report.endpoint.provider)
            .unwrap_or(planned.len())
    };
    rows.sort_by(|a, b| {
        place(a)
            .cmp(&place(b))
            .then_with(|| a.endpoint.provider.cmp(&b.endpoint.provider))
            .then_with(|| a.endpoint.model.cmp(&b.endpoint.model))
    });

    let mut page = String::from(
        "# Compatibility\n\n\
         <!-- Made by `aralo conformance table` from conformance/reports/ and \
         conformance/endpoints.toml; `make compatibility` makes it again. Edit those, \
         not this. -->\n\n\
         How each endpoint did in Aralo's conformance suite, `aralo conformance` \
         (see `conformance/README.md`). An endpoint conforms when no required protocol \
         check fails; usage and JSON output are reported and not required. A check is \
         skipped when there was nothing to check, and the notes under the table say why. \
         The evaluation column counts the snippet editor's actions the model did as \
         expected, out of the set in `conformance/evaluation.toml`: it measures the \
         model, not the endpoint, and never decides conformance.\n\n",
    );

    if rows.is_empty() {
        page.push_str("No endpoint has a report yet.\n");
    } else {
        page.push_str("| Endpoint | Model | Conforms |");
        for check in ProtocolCheck::ALL {
            page.push_str(&format!(" {} |", check.label()));
        }
        page.push_str(" First token | Evaluation | Run |\n|");
        for _ in 0..ProtocolCheck::ALL.len() + 6 {
            page.push_str(" --- |");
        }
        page.push('\n');
        for report in &rows {
            let endpoint = &report.endpoint;
            page.push_str(&format!(
                "| {} | `{}` | {} |",
                endpoint.provider,
                endpoint.model,
                if report.passed { "yes" } else { "**no**" }
            ));
            for check in ProtocolCheck::ALL {
                let cell = match report.check(check).map(|result| result.verdict) {
                    Some(Verdict::Fail) => "**fail**",
                    Some(verdict) => verdict.as_str(),
                    None => "",
                };
                page.push_str(&format!(" {cell} |"));
            }
            let first_token = report
                .first_token_ms
                .map(|ms| format!("{ms} ms"))
                .unwrap_or_default();
            let evaluation = report
                .evaluation
                .as_ref()
                .map(|evaluation| format!("{} of {}", evaluation.passed, evaluation.cases.len()))
                .unwrap_or_default();
            page.push_str(&format!(
                " {first_token} | {evaluation} | {} |\n",
                report.date
            ));
        }

        let mut notes = String::new();
        for report in &rows {
            let who = format!("{}, `{}`", report.endpoint.provider, report.endpoint.model);
            for result in &report.protocol {
                let what = match result.verdict {
                    Verdict::Pass => continue,
                    Verdict::Fail if result.required => "failed",
                    Verdict::Fail => "failed (not required)",
                    Verdict::Skip => "skipped",
                };
                notes.push_str(&format!(
                    "- {who}: **{}** {what}: {}\n",
                    result.check,
                    one_line(&result.detail)
                ));
            }
            for case in report
                .evaluation
                .iter()
                .flat_map(|evaluation| &evaluation.cases)
            {
                if case.verdict != Verdict::Pass {
                    notes.push_str(&format!(
                        "- {who}: evaluation case \u{201c}{}\u{201d}: {}\n",
                        case.case,
                        one_line(&case.detail)
                    ));
                }
            }
        }
        if !notes.is_empty() {
            page.push_str("\n## What did not pass, and why\n\n");
            page.push_str(&notes);
        }
    }

    let waiting: Vec<&PlannedEndpoint> = planned
        .iter()
        .filter(|endpoint| {
            !rows
                .iter()
                .any(|report| report.endpoint.provider == endpoint.name)
        })
        .collect();
    if !waiting.is_empty() {
        page.push_str(
            "\n## Not run live yet\n\n\
             These endpoints are in `conformance/endpoints.toml` and have no report. The \
             nightly workflow runs one once its key is a repository secret. Where a \
             recorded stream is named, the mock server replays its framing through the \
             protocol checks on every push.\n\n\
             | Endpoint | Base URL | Model | Runs | Recorded stream |\n\
             | --- | --- | --- | --- | --- |\n",
        );
        for endpoint in waiting {
            let runs = match &endpoint.key_env {
                Some(variable) => format!("{}, with `{variable}`", endpoint.runs),
                None => endpoint.runs.clone(),
            };
            let transcript = endpoint
                .transcript
                .as_deref()
                .map(|path| format!("`{path}`"))
                .unwrap_or_default();
            page.push_str(&format!(
                "| {} | `{}` | `{}` | {runs} | {transcript} |\n",
                endpoint.name, endpoint.base_url, endpoint.model
            ));
        }
    }
    page
}

/// Text for one line of Markdown: no line breaks, and no bar to split a
/// table cell.
fn one_line(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace('|', "\\|")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_evaluation_set_reads() {
        let cases = bundled_evaluation_set().unwrap();
        assert!(cases.len() >= 5);
        for case in &cases {
            case.authoring().unwrap();
        }
    }

    #[test]
    fn an_evaluation_set_that_expects_nothing_is_refused() {
        let problem =
            evaluation_set("[[case]]\nname = \"idle\"\naction = \"proofread\"\ntext = \"hi\"\n")
                .unwrap_err();
        assert!(problem.contains("expects nothing"), "{problem}");
        let problem = evaluation_set(
            "[[case]]\nname = \"odd\"\naction = \"sing\"\ntext = \"hi\"\ncontains = [\"x\"]\n",
        )
        .unwrap_err();
        assert!(problem.contains("not an editor action"), "{problem}");
    }

    fn case(text: &str) -> EvaluationCase {
        EvaluationCase {
            name: "case".into(),
            action: "proofread".into(),
            text: text.into(),
            label: String::new(),
            note: String::new(),
            language: String::new(),
            contains: Vec::new(),
            lacks: Vec::new(),
            any_of: Vec::new(),
            keeps_placeholders: false,
            at_most: None,
        }
    }

    #[test]
    fn answers_are_scored_by_whole_words_ignoring_case_and_curly_quotes() {
        let mut formal = case("hey!! cant make it thursday, cheers");
        formal.contains = vec!["friday".into()];
        formal.lacks = vec!["hey".into(), "cant".into(), "you're order".into()];
        assert_eq!(
            formal.problems("They asked whether Friday would suit you."),
            Vec::<String>::new()
        );
        assert_eq!(
            formal.problems("Hey, Friday? Thanks for you\u{2019}re order"),
            [
                "still has \u{201c}hey\u{201d}",
                "still has \u{201c}you're order\u{201d}"
            ]
        );
        assert_eq!(formal.problems("   "), ["the model wrote nothing"]);

        let mut date = case("Sent on {{date: %d %B}}, in {{field: days}} days.");
        date.keeps_placeholders = true;
        date.any_of = vec!["{{date".into()];
        assert!(date
            .problems("Sent on {{date: %d %B}}, in {{field: days}} days.")
            .is_empty());
        assert_eq!(
            date.problems("Sent on {{date}}, in {{field: days}} days."),
            ["dropped {{date: %d %B}}", "added {{date}}"]
        );
        assert_eq!(
            date.problems("Sent today."),
            [
                "has none of \u{201c}{{date\u{201d}",
                "dropped {{date: %d %B}}",
                "dropped {{field: days}}"
            ]
        );

        let mut shorter = case("one two three four five six seven eight nine ten");
        shorter.at_most = Some(0.5);
        assert!(shorter.problems("one to ten").is_empty());
        assert_eq!(
            shorter.problems("one two three four five six seven"),
            ["is 69% of the text's length, over 50%"]
        );
    }

    #[test]
    fn a_report_names_its_endpoint_by_host_and_never_holds_a_key() {
        let profile = |base_url: &str| Profile {
            name: "mine".into(),
            adapter: aralo_ai::AdapterKind::OpenAiCompat,
            base_url: base_url.into(),
            headers: Vec::new(),
            default_model: "m".into(),
            key_ref: None,
        };
        assert_eq!(
            describe(&profile("https://api.groq.com/openai/v1")),
            ("Groq".into(), "api.groq.com:443".into(), false)
        );
        assert_eq!(
            describe(&profile("http://127.0.0.1:11434/v1")),
            ("Ollama".into(), "127.0.0.1:11434".into(), true)
        );
        assert_eq!(
            describe(&profile("http://[::1]:9999/v1?key=abc")),
            ("[::1]:9999".into(), "[::1]:9999".into(), true)
        );
        assert_eq!(
            describe(&profile("https://proxy.example.com/team/v1")),
            (
                "proxy.example.com:443".into(),
                "proxy.example.com:443".into(),
                false
            )
        );
        assert_eq!(
            scrub("401: Incorrect API key provided: sk-proj-4f9aQ2mZ7xKc81LwPq0RtYb3."),
            "401: Incorrect API key provided: [redacted]"
        );
    }
}
