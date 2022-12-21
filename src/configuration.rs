//
//  flock - baa (with twenty instances of the letter "a")
//  Copyright (C) superwhiskers <whiskerdev@protonmail.com> 2022
//
//  This program is free software: you can redistribute it and/or modify
//  it under the terms of the GNU Affero General Public License as published by
//  the Free Software Foundation, either version 3 of the License, or
//  (at your option) any later version.
//
//  This program is distributed in the hope that it will be useful,
//  but WITHOUT ANY WARRANTY; without even the implied warranty of
//  MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
//  GNU Affero General Public License for more details.
//
//  You should have received a copy of the GNU Affero General Public License
//  along with this program.  If not, see <https://www.gnu.org/licenses/>.
//

use config::{Config, ConfigError, Environment, File};
use serde::Deserialize;
use std::{
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    path::PathBuf,
};

/// The main configuration structure
#[derive(Deserialize, Clone, Debug, Eq, PartialEq)]
pub struct Configuration {
    /// The `general` section of the configuration
    #[serde(default)]
    pub general: General,

    /// The `http` section of the configuration
    #[serde(default)]
    pub http: Http,

    /// The `sqlite` section of the configuration
    #[serde(default)]
    pub sqlite: Sqlite,

    /// The `route` section of the configuration
    #[serde(default)]
    pub routes: Routes,
}

impl Configuration {
    pub fn new() -> Result<Self, ConfigError> {
        Config::builder()
            .add_source(File::with_name("config.toml").required(false))
            .add_source(
                Environment::with_prefix("flock")
                    .separator("__")
                    .list_separator(","),
            )
            .build()?
            .try_deserialize()
    }
}

/// The structure representing the `general` section of the configuration
#[derive(Deserialize, Clone, Debug, Eq, PartialEq)]
pub struct General {
    /// The logger's filter configuration
    #[serde(default = "default_log_filter")]
    pub log_filter: String,
}

impl Default for General {
    fn default() -> Self {
        Self {
            log_filter: default_log_filter(),
        }
    }
}

/// The default value for the `log_filter` field in the [`General`] configuration section
#[inline(always)]
fn default_log_filter() -> String {
    "info".to_string()
}

/// The structure representing the `http` section of the configuration
#[derive(Deserialize, Clone, Debug, Eq, PartialEq)]
pub struct Http {
    /// The configuration for TLS
    pub tls: Option<Tls>,

    /// The address to listen on
    #[serde(default = "default_address")]
    pub address: SocketAddr,
}

impl Default for Http {
    fn default() -> Self {
        Self {
            tls: None,
            address: default_address(),
        }
    }
}

/// The default value for the `address` field in the [`Http`] configuration section
#[inline(always)]
fn default_address() -> SocketAddr {
    SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 8080))
}

/// The structure representing the `http.tls` section of the configuration
#[derive(Deserialize, Clone, Debug, Eq, PartialEq)]
pub struct Tls {
    /// The path to the certificate used to host the server
    pub certificate: String,

    /// The path to the certificate's corresponding key
    pub key: String,
}

/// The structure representing the `sqlite` section of the configuration
#[derive(Deserialize, Clone, Debug, Eq, PartialEq)]
pub struct Sqlite {
    /// The path to the sqlite database file
    #[serde(default = "default_path")]
    pub path: PathBuf,

    /// Whether or not to create the database file if it doesn't exist
    #[serde(default = "default_create_if_missing")]
    pub create_if_missing: bool,

    /// The minimum number of connections to have open to the database
    pub min_connections: Option<u32>,

    /// The maximum number of connections to have open to the database
    pub max_connections: Option<u32>,
}

impl Default for Sqlite {
    fn default() -> Self {
        Self {
            path: default_path(),
            create_if_missing: default_create_if_missing(),
            min_connections: None,
            max_connections: None,
        }
    }
}

/// The default value for the `path` field in the [`Sqlite`] configuration section
#[inline(always)]
fn default_path() -> PathBuf {
    "flock.db"
        .parse()
        .expect("this conversion should be infallible")
}

/// The default value for the `create_if_missing` field in the [`Sqlite`] configuration
/// section
#[inline(always)]
fn default_create_if_missing() -> bool {
    true
}

/// Configuration pertaining specifically to how routes are responded to
#[derive(Deserialize, Clone, Debug, Eq, PartialEq)]
pub struct Routes {
    /// Whether or not to enforce that cookies be set only to secure origins
    #[serde(default = "default_secure_cookies")]
    pub secure_cookies: bool,
}

impl Default for Routes {
    fn default() -> Self {
        Self {
            secure_cookies: default_secure_cookies(),
        }
    }
}

/// The default value for the `secure_cookies` field in the [`Routes`] configuration section
#[inline(always)]
fn default_secure_cookies() -> bool {
    false
}
