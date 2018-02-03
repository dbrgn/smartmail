use std::env;


#[derive(Debug)]
pub struct Config {
    pub ttn_app_id: String,
    pub ttn_access_key: String,

    pub threema_from: String,
    pub threema_to: Vec<String>,
    pub threema_secret: String,
    pub threema_private_key: String,

    pub influxdb: Option<InfluxConfig>,
}

#[derive(Debug)]
pub struct InfluxConfig {
    pub user: String,
    pub pass: String,
    pub db: String,
    pub url: String,
}

fn get_env_var(name: &str) -> Result<String, String> {
    env::var(name).map_err(|_| format!("Missing {} env var", name))
}

impl Config {
    pub fn init() -> Result<Config, String> {
        let influx_user = env::var("INFLUXDB_USER").ok();
        let influx_pass = env::var("INFLUXDB_PASS").ok();
        let influx_db = env::var("INFLUXDB_DB").ok();
        let influx_url = env::var("INFLUXDB_URL").ok();
        let influxdb = match (influx_user, influx_pass, influx_db, influx_url) {
            (Some(user), Some(pass), Some(db), Some(url)) => Some(InfluxConfig { user, pass, db, url }),
            _ => None,
        };

        Ok(Config {
            ttn_app_id: get_env_var("TTN_APP_ID")?,
            ttn_access_key: get_env_var("TTN_ACCESS_KEY")?,
            threema_from: get_env_var("THREEMA_FROM")?,
            threema_to: get_env_var("THREEMA_TO")?.split(',').map(|s| s.to_owned()).collect(),
            threema_secret: get_env_var("THREEMA_SECRET")?,
            threema_private_key: get_env_var("THREEMA_PRIVATE_KEY")?,
            influxdb: influxdb,
        })
    }
}
