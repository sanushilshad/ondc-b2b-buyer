use config::{self, ConfigError, Environment};
use secrecy::{ExposeSecret, Secret};
use serde::Deserialize;
use sqlx::{postgres::PgConnectOptions, ConnectOptions};

use crate::domain::SubscriberEmail;

#[derive(Debug, Deserialize)]
pub struct Settings {
    pub database: DatabaseSettings,
    pub application_port: u16,
    pub email_client: EmailClientSettings,
}

#[derive(Debug, Deserialize)]
pub struct DatabaseSettings {
    pub username: String,
    pub password: Secret<String>,
    pub port: u16,
    pub host: String,
    pub name: String,
}

impl DatabaseSettings {
    // Renamed from `connection_string_without_db`
    pub fn without_db(&self) -> PgConnectOptions {
        PgConnectOptions::new()
            .host(&self.host)
            .username(&self.username)
            .password(&self.password.expose_secret())
            .port(self.port)
    }
    // Renamed from `connection_string`
    pub fn with_db(&self) -> PgConnectOptions {
        let mut options = self.without_db().database(&self.name);
        options.log_statements(tracing::log::LevelFilter::Trace);
        options
    }

    // pub fn from_env() -> Result<Self, DatabaseError> {
    //     let username: String = "postgres".to_string();

    //     let database_name = "rapidor_master".to_string();

    //     let password: String = std::env::var("RAPIDOR_DB_PASSWORD")
    //         .map_err(|_| DatabaseError::MissingDatabasePassword)?;
    //     println!("{:?}", &password);
    //     let port: u16 = std::env::var("RAPIDOR_DB_PORT")
    //         .map_err(|_| DatabaseError::MissingDatabasePort)?
    //         .parse()
    //         .map_err(|_| DatabaseError::DatabasePortMustbeNumber)?;

    //     let host: String =
    //         std::env::var("RAPIDOR_DB_IP").map_err(|_| DatabaseError::MissingDatabaseIP)?;
    //     let password_secret = Secret::new(password);
    //     Ok(DatabaseSettings {
    //         username,
    //         password: password_secret,
    //         port,
    //         host,
    //         name: database_name,
    //     })
    // }
}

#[derive(Debug, Deserialize)]
pub struct EmailClientSettings {
    pub base_url: String,
    pub username: String,
    pub password: Secret<String>,
    pub sender_email: String,
    pub timeout_milliseconds: u64,
}
impl EmailClientSettings {
    pub fn sender(&self) -> Result<SubscriberEmail, String> {
        SubscriberEmail::parse(self.sender_email.clone())
    }

    pub fn timeout(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.timeout_milliseconds)
    }
}

pub fn get_configuration() -> Result<Settings, ConfigError> {
    let base_path = std::env::current_dir().expect("Failed to determine the current directory");
    let configuration_directory = base_path.join("configuration");
    let builder = config::Config::builder()
        .add_source(config::File::from(
            configuration_directory.join("configuration.yaml"),
        ))
        .add_source(Environment::default().separator("__"))
        .build()?;
    builder.try_deserialize::<Settings>()
}