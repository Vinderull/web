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
        Ok(Self {
            bind_addr: env::var("BIND_ADDR")
                .unwrap_or_else(|_| format!("{}:3000", Ipv4Addr::UNSPECIFIED)),
            content_dir: PathBuf::from(
                env::var("CONTENT_DIR").unwrap_or_else(|_| "content".into()),
            ),
            static_dir: PathBuf::from(
                env::var("STATIC_DIR").unwrap_or_else(|_| "static".into()),
            ),
        })
    }
}
