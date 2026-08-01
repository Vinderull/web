use std::env;
use std::net::Ipv4Addr;
use std::path::PathBuf;

use anyhow::Result;

pub struct Config {
    pub bind_addr: String,
    pub content_dir: PathBuf,
    pub static_dir: PathBuf,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        Ok(Self::resolve(
            env::var("BIND_ADDR").ok(),
            env::var("CONTENT_DIR").ok(),
            env::var("STATIC_DIR").ok(),
        ))
    }

    /// Resolve config from optional overrides, falling back to defaults.
    /// Pure (no env access) so it's unit-testable without mutating process env.
    fn resolve(bind: Option<String>, content: Option<String>, static_dir: Option<String>) -> Self {
        Self {
            bind_addr: bind.unwrap_or_else(|| format!("{}:3000", Ipv4Addr::UNSPECIFIED)),
            content_dir: PathBuf::from(content.unwrap_or_else(|| "content".into())),
            static_dir: PathBuf::from(static_dir.unwrap_or_else(|| "static".into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_unset() {
        let c = Config::resolve(None, None, None);
        assert_eq!(c.bind_addr, "0.0.0.0:3000");
        assert_eq!(c.content_dir, PathBuf::from("content"));
        assert_eq!(c.static_dir, PathBuf::from("static"));
    }

    #[test]
    fn overrides_when_set() {
        let c = Config::resolve(
            Some("127.0.0.1:8080".into()),
            Some("/data/content".into()),
            Some("/data/static".into()),
        );
        assert_eq!(c.bind_addr, "127.0.0.1:8080");
        assert_eq!(c.content_dir, PathBuf::from("/data/content"));
        assert_eq!(c.static_dir, PathBuf::from("/data/static"));
    }

    #[test]
    fn partial_overrides_preserve_defaults() {
        let c = Config::resolve(Some(":4000".into()), None, None);
        assert_eq!(c.bind_addr, ":4000");
        assert_eq!(c.content_dir, PathBuf::from("content"));
        assert_eq!(c.static_dir, PathBuf::from("static"));
    }
}
