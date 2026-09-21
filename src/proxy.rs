//! Proxy list loading.

use std::path::Path;

use anyhow::{Context, Result};
use rand::seq::IndexedRandom;

/// Environment variable overriding the proxy file path.
pub const PROXIES_FILE_ENV: &str = "PROXIES_FILE";
/// Default proxy file path.
pub const PROXIES_FILE_DEFAULT: &str = "proxies.txt";

/// Rotating list of `ip:port:user:pass` proxies.
#[derive(Debug, Default)]
pub struct ProxyList {
    urls: Vec<String>,
}

impl ProxyList {
    /// Loads proxies from `path`. A missing file yields an empty list.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;

        let mut urls = Vec::new();
        for (index, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            urls.push(parse(line).with_context(|| format!("{}:{}", path.display(), index + 1))?);
        }

        Ok(Self { urls })
    }

    /// Number of loaded proxies.
    pub fn len(&self) -> usize {
        self.urls.len()
    }

    /// Whether no proxies were loaded.
    pub fn is_empty(&self) -> bool {
        self.urls.is_empty()
    }

    /// Picks a random proxy URL.
    pub fn random(&self) -> Option<&str> {
        self.urls.choose(&mut rand::rng()).map(String::as_str)
    }
}

/// Parses `ip:port:user:pass` (user/pass optional) into a proxy URL.
fn parse(line: &str) -> Result<String> {
    let mut fields = line.splitn(4, ':');
    let host = fields.next().context("missing host")?;
    let port = fields.next().context("missing port")?;

    let mut url = url::Url::parse(&format!("http://{host}:{port}"))?;
    if let Some(user) = fields.next() {
        url.set_username(user)
            .map_err(|()| anyhow::anyhow!("invalid proxy username"))?;
    }
    if let Some(pass) = fields.next() {
        url.set_password(Some(pass))
            .map_err(|()| anyhow::anyhow!("invalid proxy password"))?;
    }
    Ok(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_and_short_forms() {
        assert_eq!(
            parse("1.2.3.4:8080:alice:s3cret").unwrap(),
            "http://alice:s3cret@1.2.3.4:8080/"
        );
        assert_eq!(parse("5.6.7.8:3128").unwrap(), "http://5.6.7.8:3128/");
    }

    #[test]
    fn load_skips_comments_and_blanks() {
        let path = std::env::temp_dir().join(format!("proxies-test-{}.txt", rand::random::<u64>()));
        std::fs::write(&path, "# comment\n1.2.3.4:8080:a:b\n\n5.6.7.8:3128\n").unwrap();

        let list = ProxyList::load(&path).unwrap();
        assert_eq!(list.len(), 2);

        std::fs::remove_file(&path).unwrap();
    }
}
