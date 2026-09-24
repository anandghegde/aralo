//! The first stage: whether a request may be made at all.

use std::net::IpAddr;

use crate::error::Refusal;
use crate::model::Profile;

/// The AI switch, local-only mode and any managed restriction. The default is
/// everything off: AI is optional, and a fresh install makes no model call.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Policy {
    /// The master switch. Off, no model call is possible.
    pub ai_enabled: bool,
    /// Only endpoints on this Mac (PRD P6).
    pub local_only: bool,
    /// Set by a managed profile: the only hosts a request may go to. An entry
    /// is a host name, or `*.` and a domain for any name under it.
    pub allowed_hosts: Option<Vec<String>>,
}

impl Policy {
    /// Checks a profile against the policy and returns where it points. This
    /// runs before context is gathered, so a refused request never asks the
    /// shell for the selection or the clipboard.
    pub fn check(&self, profile: &Profile) -> Result<Endpoint, Refusal> {
        if !self.ai_enabled {
            return Err(Refusal::Off);
        }
        let endpoint = crate::guard::parse_endpoint(&profile.base_url)?;
        if self.local_only && !endpoint.host.is_loopback() {
            return Err(Refusal::LocalOnly {
                host: endpoint.host.to_string(),
            });
        }
        if let Some(allowed) = &self.allowed_hosts {
            if !allowed.iter().any(|pattern| endpoint.host.matches(pattern)) {
                return Err(Refusal::Managed {
                    host: endpoint.host.to_string(),
                });
            }
        }
        Ok(endpoint)
    }
}

/// Where a profile points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub https: bool,
    pub host: Host,
    pub port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Host {
    /// A name, lower case.
    Name(String),
    Ip(IpAddr),
}

impl Host {
    /// Whether the host is this machine by what it says. For a name that is
    /// `localhost` and the names under it (RFC 6761); the network guard still
    /// checks the addresses such a name resolves to before connecting.
    pub fn is_loopback(&self) -> bool {
        match self {
            Self::Ip(ip) => ip.to_canonical().is_loopback(),
            Self::Name(name) => name == "localhost" || name.ends_with(".localhost"),
        }
    }

    fn matches(&self, pattern: &str) -> bool {
        let pattern = pattern.trim().to_ascii_lowercase();
        match self {
            Self::Ip(ip) => pattern
                .parse::<IpAddr>()
                .is_ok_and(|allowed| allowed == *ip),
            Self::Name(name) => match pattern.strip_prefix("*.") {
                Some(domain) => name
                    .strip_suffix(domain)
                    .is_some_and(|rest| rest.ends_with('.')),
                None => *name == pattern,
            },
        }
    }
}

impl std::fmt::Display for Host {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Name(name) => f.write_str(name),
            Self::Ip(ip) => write!(f, "{ip}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::AdapterKind;

    fn profile(url: &str) -> Profile {
        Profile {
            name: "test".into(),
            adapter: AdapterKind::OpenAiCompat,
            base_url: url.into(),
            headers: Vec::new(),
            default_model: "m".into(),
            key_ref: None,
        }
    }

    fn on() -> Policy {
        Policy {
            ai_enabled: true,
            ..Policy::default()
        }
    }

    #[test]
    fn the_default_is_off() {
        assert_eq!(
            Policy::default().check(&profile("http://127.0.0.1:11434/v1")),
            Err(Refusal::Off)
        );
    }

    #[test]
    fn local_only_takes_loopback_in_every_spelling() {
        let policy = Policy {
            local_only: true,
            ..on()
        };
        for url in [
            "http://127.0.0.1:11434/v1",
            "http://127.8.9.10/v1",
            "http://[::1]:1234/v1",
            "http://[::ffff:127.0.0.1]/v1",
            "http://localhost:8080/v1",
            "http://LOCALHOST:8080/v1",
            "http://ollama.localhost/v1",
        ] {
            assert!(policy.check(&profile(url)).is_ok(), "{url} is this Mac");
        }
        for url in [
            "https://api.openai.com/v1",
            "http://192.168.1.20:11434/v1",
            "http://[::ffff:10.0.0.1]/v1",
            "http://localhost.example.com/v1",
            "http://0.0.0.0:11434/v1",
        ] {
            assert!(
                matches!(policy.check(&profile(url)), Err(Refusal::LocalOnly { .. })),
                "{url} is not this Mac"
            );
        }
    }

    #[test]
    fn a_managed_list_names_hosts_and_domains() {
        let policy = Policy {
            allowed_hosts: Some(vec!["api.openai.com".into(), "*.corp.example".into()]),
            ..on()
        };
        assert!(policy.check(&profile("https://api.openai.com/v1")).is_ok());
        assert!(policy
            .check(&profile("https://llm.corp.example/v1"))
            .is_ok());
        for url in [
            "https://api.anthropic.com/v1",
            "https://corp.example/v1",
            "https://evilcorp.example/v1",
        ] {
            assert!(
                matches!(policy.check(&profile(url)), Err(Refusal::Managed { .. })),
                "{url}"
            );
        }
    }

    #[test]
    fn a_url_that_is_not_http_is_refused() {
        for url in ["file:///etc/passwd", "ftp://example.com", "not a url", ""] {
            assert!(
                matches!(on().check(&profile(url)), Err(Refusal::BadUrl(_))),
                "{url}"
            );
        }
    }
}
