//! Keychain-backed secrets, per-source cookies, and fetch-plan validation.
//!
//! Secrets live in the OS keychain (Secret Service on Linux, Credential Manager
//! on Windows) via the `keyring` crate. NEVER Stronghold (deprecated for v3) and
//! never the DB or settings files.

use crate::sources::{FetchPlan, SourceDescriptor};

const SERVICE: &str = "dev.colewiz.wizsearch";

#[derive(Debug, thiserror::Error)]
pub enum SecurityError {
    #[error("keychain: {0}")]
    Keychain(String),
    #[error("fetch plan rejected: {0}")]
    PlanRejected(String),
}

impl From<keyring::Error> for SecurityError {
    fn from(e: keyring::Error) -> Self {
        SecurityError::Keychain(e.to_string())
    }
}

// ---------- keychain ----------

fn entry(name: &str) -> Result<keyring::Entry, SecurityError> {
    Ok(keyring::Entry::new(SERVICE, name)?)
}

/// keychain entry name for a source's API key/token
pub fn credential_key(source_id: &str) -> String {
    format!("source.{source_id}.api_key")
}

/// keychain entry name for a source's login cookies
pub fn cookie_key(source_id: &str) -> String {
    format!("source.{source_id}.cookies")
}

pub fn secret_set(name: &str, value: &str) -> Result<(), SecurityError> {
    entry(name)?.set_password(value)?;
    Ok(())
}

pub fn secret_get(name: &str) -> Result<Option<String>, SecurityError> {
    match entry(name)?.get_password() {
        Ok(v) => Ok(Some(v)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// async-safe read: the secret-service backend blocks on dbus and panics if
/// called on a tokio runtime thread, so hop to a blocking thread first
pub async fn secret_get_async(name: String) -> Result<Option<String>, SecurityError> {
    tokio::task::spawn_blocking(move || secret_get(&name))
        .await
        .map_err(|e| SecurityError::Keychain(e.to_string()))?
}

pub fn secret_clear(name: &str) -> Result<(), SecurityError> {
    match entry(name)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

// ---------- startup assertions ----------

/// Hard invariant: WizSearch never ships a developer API key. Every source
/// declares its embedded credential, and it must be empty. Panics on violation
/// so a bad build cannot start.
pub fn assert_no_embedded_credentials(descriptors: &[&SourceDescriptor]) {
    for d in descriptors {
        assert!(
            d.embedded_credential.is_empty(),
            "source '{}' ships an embedded credential; WizSearch never embeds keys",
            d.id
        );
    }
}

// ---------- URL / fetch-plan validation ----------

/// host allowlist check: exact match or subdomain of an allowed suffix
fn host_allowed(host: &str, allowed: &[&str]) -> bool {
    let host = host.to_ascii_lowercase();
    allowed.iter().any(|a| {
        let a = a.to_ascii_lowercase();
        host == a || host.ends_with(&format!(".{a}"))
    })
}

/// Validate any URL a source handed us before the host touches it: https only,
/// no userinfo, no weird ports, host on the source's allowlist.
pub fn validate_source_url(raw: &str, desc: &SourceDescriptor) -> Result<url::Url, SecurityError> {
    let parsed =
        url::Url::parse(raw).map_err(|e| SecurityError::PlanRejected(format!("bad url: {e}")))?;
    if parsed.scheme() != "https" {
        return Err(SecurityError::PlanRejected(format!(
            "scheme '{}' not allowed (https only)",
            parsed.scheme()
        )));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(SecurityError::PlanRejected("userinfo in url".into()));
    }
    if parsed.port().is_some() {
        return Err(SecurityError::PlanRejected("non-default port".into()));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| SecurityError::PlanRejected("no host".into()))?;
    if !host_allowed(host, desc.allowed_hosts) {
        return Err(SecurityError::PlanRejected(format!(
            "host '{host}' not in {}'s allowlist",
            desc.id
        )));
    }
    Ok(parsed)
}

/// Validate a FetchPlan right before execution (which only happens on explicit
/// user selection).
pub fn validate_fetch_plan(plan: &FetchPlan, desc: &SourceDescriptor) -> Result<(), SecurityError> {
    match plan {
        FetchPlan::HttpGet { url, headers, .. } => {
            validate_source_url(url, desc)?;
            for (name, _) in headers {
                let lower = name.to_ascii_lowercase();
                // sources must not smuggle auth or cookies through plan headers
                if lower == "authorization" || lower == "cookie" {
                    return Err(SecurityError::PlanRejected(format!(
                        "plan header '{name}' not allowed"
                    )));
                }
            }
            Ok(())
        }
        FetchPlan::YtDlp { url, .. } => {
            validate_source_url(url, desc)?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::AssetType;

    static DESC: SourceDescriptor = SourceDescriptor {
        id: "test",
        name: "Test",
        homepage: "https://example.com",
        asset_types: &[AssetType::Audio],
        requires_key: false,
        key_help_url: "",
        allowed_hosts: &["example.com"],
        default_rate_limit_per_min: 10,
        embedded_credential: "",
    };

    #[test]
    fn allows_subdomains_only_of_allowlist() {
        assert!(validate_source_url("https://cdn.example.com/a.mp3", &DESC).is_ok());
        assert!(validate_source_url("https://example.com/a.mp3", &DESC).is_ok());
        assert!(validate_source_url("https://evilexample.com/a.mp3", &DESC).is_err());
        assert!(validate_source_url("https://example.com.evil.io/a.mp3", &DESC).is_err());
    }

    #[test]
    fn rejects_http_userinfo_ports() {
        assert!(validate_source_url("http://example.com/a.mp3", &DESC).is_err());
        assert!(validate_source_url("https://u:p@example.com/a.mp3", &DESC).is_err());
        assert!(validate_source_url("https://example.com:8443/a.mp3", &DESC).is_err());
    }

    #[test]
    fn rejects_auth_headers_in_plans() {
        let plan = FetchPlan::HttpGet {
            url: "https://example.com/a.mp3".into(),
            headers: vec![("Authorization".into(), "Bearer x".into())],
            filename_hint: "a.mp3".into(),
        };
        assert!(validate_fetch_plan(&plan, &DESC).is_err());
    }

    #[test]
    fn empty_embedded_credentials_pass() {
        assert_no_embedded_credentials(&[&DESC]);
    }
}
