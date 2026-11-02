//! HTTPS requests, with the limits the app needs: https only, known hosts only, a size limit on every
//! download, and the app's name as User-Agent (HearthstoneJSON refuses anonymous requests).

use std::io::Read;
use std::time::Duration;

use ureq::Agent;

/// Hosts the app talks to. Anything else is refused before any connection.
const ALLOWED: &[&str] = &[
    "https://api.hearthstonejson.com/",
    "https://art.hearthstonejson.com/",
    "https://api.github.com/repos/owlingstone/owlingstone/",
    "https://github.com/owlingstone/owlingstone/releases/download/",
    "https://owlingstone-api.mimizuku-dawn.workers.dev/",
];

pub fn user_agent() -> String {
    format!("Owlingstone/{} (+https://github.com/owlingstone/owlingstone)", crate::VERSION)
}

fn agent(timeout_secs: u64) -> Agent {
    Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(timeout_secs)))
        .http_status_as_error(false)
        .build()
        .into()
}

fn check(url: &str) -> Result<(), String> {
    if ALLOWED.iter().any(|p| url.starts_with(p)) {
        Ok(())
    } else {
        Err(format!("unexpected URL {url}"))
    }
}

/// The body of a GET, at most `limit` bytes (more is an error, never a cut file).
pub fn get(url: &str, limit: u64, timeout_secs: u64) -> Result<Vec<u8>, String> {
    get_with(url, limit, timeout_secs, None)
}

/// Same as `get`, for the GitHub API (asks for its JSON format).
pub fn get_github(url: &str, limit: u64, timeout_secs: u64) -> Result<Vec<u8>, String> {
    get_with(url, limit, timeout_secs, Some("application/vnd.github+json"))
}

fn get_with(url: &str, limit: u64, timeout_secs: u64, accept: Option<&str>) -> Result<Vec<u8>, String> {
    check(url)?;
    let mut req = agent(timeout_secs).get(url).header("User-Agent", &user_agent());
    if let Some(a) = accept {
        req = req.header("Accept", a);
    }
    let mut resp = req.call().map_err(|e| format!("{url}: {e}"))?;
    let status = resp.status().as_u16();
    if status != 200 {
        return Err(format!("{url}: HTTP {status}"));
    }
    read_limited(resp.body_mut().as_reader(), limit).map_err(|e| format!("{url}: {e}"))
}

/// Reads at most `limit` bytes; more is an error.
pub fn read_limited(reader: impl Read, limit: u64) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    reader.take(limit + 1).read_to_end(&mut out).map_err(|e| e.to_string())?;
    if out.len() as u64 > limit {
        return Err(format!("larger than {limit} bytes"));
    }
    Ok(out)
}

/// Streams a GET to a file (big downloads never sit in memory). Returns the number of bytes.
pub fn download_to(url: &str, path: &std::path::Path, limit: u64, timeout_secs: u64) -> Result<u64, String> {
    check(url)?;
    let mut resp = agent(timeout_secs)
        .get(url)
        .header("User-Agent", &user_agent())
        .call()
        .map_err(|e| format!("{url}: {e}"))?;
    let status = resp.status().as_u16();
    if status != 200 {
        return Err(format!("{url}: HTTP {status}"));
    }
    let mut file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut reader = resp.body_mut().with_config().limit(limit + 1).reader();
    let n = std::io::copy(&mut reader, &mut file).map_err(|e| e.to_string())?;
    if n > limit {
        return Err(format!("{url} is larger than {limit} bytes"));
    }
    Ok(n)
}

/// Sends JSON (POST) or nothing (DELETE) and returns the HTTP status. Network errors are `Err`.
pub fn send(method: &str, url: &str, body: Option<&serde_json::Value>, timeout_secs: u64) -> Result<u16, String> {
    check(url)?;
    let agent = agent(timeout_secs);
    let ua = user_agent();
    let resp = match (method, body) {
        ("POST", Some(b)) => {
            let bytes = serde_json::to_vec(b).map_err(|e| e.to_string())?;
            agent.post(url).header("User-Agent", &ua).header("content-type", "application/json").send(&bytes[..])
        }
        ("DELETE", None) => agent.delete(url).header("User-Agent", &ua).call(),
        _ => return Err(format!("unsupported request {method}")),
    };
    resp.map(|r| r.status().as_u16()).map_err(|e| format!("{url}: {e}"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn unknown_hosts_are_refused() {
        assert!(super::check("https://example.com/x").is_err());
        assert!(super::check("http://api.hearthstonejson.com/v1").is_err());
        assert!(super::check("https://api.hearthstonejson.com/v1/latest/enUS/cards.json").is_ok());
        assert!(super::read_limited(&b"12345"[..], 4).is_err());
        assert_eq!(super::read_limited(&b"1234"[..], 4).unwrap(), b"1234");
    }
}
