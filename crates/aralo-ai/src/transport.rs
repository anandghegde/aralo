//! The narrow HTTP interface adapters are given. Adapters build requests and
//! read answers; they never hold a client, so they can never make one. The
//! network guard is the one real [`Transport`]; tests replay recorded
//! transcripts through their own.

use std::future::Future;
use std::pin::Pin;

use crate::error::AiError;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
}

/// An answer whose status and headers have arrived. The body streams.
/// Dropping it closes the connection, which is how a cancel stops a model.
pub struct HttpResponse {
    pub status: u16,
    /// Names are lower case.
    pub headers: Vec<(String, String)>,
    pub body: Box<dyn ByteStream>,
}

impl HttpResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

impl std::fmt::Debug for HttpResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpResponse")
            .field("status", &self.status)
            .field("headers", &self.headers)
            .finish_non_exhaustive()
    }
}

pub trait ByteStream: Send {
    /// The next chunk of the body, or `None` at its end.
    fn next_chunk(&mut self) -> BoxFuture<'_, Result<Option<Vec<u8>>, AiError>>;
}

pub trait Transport: Send + Sync {
    fn send(&self, request: HttpRequest) -> BoxFuture<'_, Result<HttpResponse, AiError>>;
}
