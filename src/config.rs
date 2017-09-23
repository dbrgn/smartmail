use std::env;


pub struct Config {
    pub ttn_app_id: String,
    pub ttn_access_key: String,
}

impl Config {
    pub fn init() -> Result<Config, String> {
        Ok(Config {
            ttn_app_id: env::var("TTN_APP_ID").map_err(|_| "Missing TTN_APP_ID env var")?,
            ttn_access_key: env::var("TTN_ACCESS_KEY").map_err(|_| "Missing TTN_ACCESS_KEY env var")?,
        })
    }
}
