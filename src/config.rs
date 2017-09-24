use std::env;


pub struct Config {
    pub ttn_app_id: String,
    pub ttn_access_key: String,

    pub threema_from: String,
    pub threema_to: Vec<String>,
    pub threema_secret: String,
    pub threema_private_key: String,
}

fn get_env_var(name: &str) -> Result<String, String> {
    env::var(name).map_err(|_| format!("Missing {} env var", name))
}

impl Config {
    pub fn init() -> Result<Config, String> {
        Ok(Config {
            ttn_app_id: get_env_var("TTN_APP_ID")?,
            ttn_access_key: get_env_var("TTN_ACCESS_KEY")?,
            threema_from: get_env_var("THREEMA_FROM")?,
            threema_to: get_env_var("THREEMA_TO")?.split(',').map(|s| s.to_owned()).collect(),
            threema_secret: get_env_var("THREEMA_SECRET")?,
            threema_private_key: get_env_var("THREEMA_PRIVATE_KEY")?,
        })
    }
}
