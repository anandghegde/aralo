//! What every adapter does with HTTP: build a URL under a profile's base,
//! read a bounded body, and turn an error status into an [`AiError`].

use std::time::Duration;

use aralo_ai::{AiError, HttpResponse};

/// An error body longer than this is cut. Only its message is kept anyway.
const MAX_ERROR_BODY: usize = 64 * 1024;
/// A whole JSON answer, such as a model list or a response that did not
/// stream, may be this long.
pub(crate) const MAX_JSON_BODY: usize = 8 * 1024 * 1024;
/// An error message shown to the user is cut to this many characters.
const MAX_MESSAGE: usize = 300;

/// `base` and `path` joined by exactly one slash.
pub(crate) fn join(base: &str, path: &str) -> String {
    format!(
        "{}/{}",
        base.trim().trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

/// Reads a body up to `limit` bytes. Anything past it is left unread, and
/// dropping the response closes the connection.
pub(crate) async fn read_body(
    response: &mut HttpResponse,
    limit: usize,
) -> Result<Vec<u8>, AiError> {
    let mut body = Vec::new();
    while let Some(chunk) = response.body.next_chunk().await? {
        body.extend_from_slice(&chunk);
        if body.len() >= limit {
            body.truncate(limit);
            break;
        }
    }
    Ok(body)
}

/// The error an endpoint answered with, as the user will read it. A body
/// that is not JSON, such as a proxy's HTML page, is summarised rather than
/// shown.
pub(crate) async fn status_error(mut response: HttpResponse) -> AiError {
    let status = response.status;
    let retry_after = response.header("retry-after").and_then(parse_retry_after);
    let body = read_body(&mut response, MAX_ERROR_BODY)
        .await
        .unwrap_or_default();
    AiError::Status {
        status,
        message: error_message(&body).unwrap_or_else(|| default_message(status).into()),
        retry_after,
    }
}

/// The message in an error body. It understands the shapes the providers
/// use: `{"error": {"message": ..}}`, `{"error": ".."}`, `{"message": ..}`
/// and `{"detail": ..}`.
pub(crate) fn error_message(body: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    message_in(&value).map(|message| clip(message.trim()))
}

pub(crate) fn message_in(value: &serde_json::Value) -> Option<&str> {
    let error = value.get("error").unwrap_or(value);
    [error.get("message"), error.get("detail"), Some(error)]
        .into_iter()
        .flatten()
        .find_map(|candidate| candidate.as_str().filter(|text| !text.trim().is_empty()))
}

fn clip(message: &str) -> String {
    match message.char_indices().nth(MAX_MESSAGE) {
        Some((end, _)) => format!("{}…", &message[..end]),
        None => message.to_owned(),
    }
}

fn default_message(status: u16) -> &'static str {
    match status {
        400 => "the request was refused as malformed",
        401 => "the key was not accepted",
        403 => "the key is not allowed to do this",
        404 => "nothing answers at this address; check the base URL and the model name",
        408 | 504 => "the endpoint timed out",
        413 => "the request is too large for this endpoint",
        429 => "rate limited",
        500..=599 => "the endpoint had a server error",
        _ => "the endpoint refused the request",
    }
}

/// `Retry-After` in seconds. The HTTP-date form is not used by any model
/// provider, and is ignored.
fn parse_retry_after(value: &str) -> Option<Duration> {
    let seconds: f64 = value.trim().parse().ok()?;
    (seconds.is_finite() && seconds >= 0.0).then(|| Duration::from_secs_f64(seconds))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_join_with_one_slash() {
        for base in [
            "https://api.openai.com/v1",
            "https://api.openai.com/v1/",
            " https://api.openai.com/v1// ",
        ] {
            assert_eq!(
                join(base, "chat/completions"),
                "https://api.openai.com/v1/chat/completions"
            );
            assert_eq!(join(base, "/models"), "https://api.openai.com/v1/models");
        }
    }

    #[test]
    fn error_messages_are_found_in_every_shape() {
        for (body, expected) in [
            (
                r#"{"error":{"message":"Incorrect API key provided","type":"invalid_request_error"}}"#,
                Some("Incorrect API key provided"),
            ),
            (
                r#"{"error":"model 'llama9' not found"}"#,
                Some("model 'llama9' not found"),
            ),
            (
                r#"{"message":"Too many requests"}"#,
                Some("Too many requests"),
            ),
            (r#"{"detail":"Not Found"}"#, Some("Not Found")),
            (r#"{"error":{"code":500}}"#, None),
            ("<html>Bad gateway</html>", None),
        ] {
            assert_eq!(
                error_message(body.as_bytes()).as_deref(),
                expected,
                "{body}"
            );
        }
        let long = format!(r#"{{"error":"{}"}}"#, "x".repeat(1000));
        assert_eq!(
            error_message(long.as_bytes()).unwrap().chars().count(),
            MAX_MESSAGE + 1
        );
    }

    #[test]
    fn retry_after_is_read_in_seconds() {
        assert_eq!(parse_retry_after("2"), Some(Duration::from_secs(2)));
        assert_eq!(parse_retry_after(" 0.5 "), Some(Duration::from_millis(500)));
        assert_eq!(parse_retry_after("Wed, 21 Oct 2015 07:28:00 GMT"), None);
        assert_eq!(parse_retry_after("-1"), None);
    }
}
