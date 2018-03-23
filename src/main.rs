extern crate byteorder;
extern crate data_encoding;
extern crate dotenv;
extern crate env_logger;
#[macro_use] extern crate lazy_static;
#[macro_use] extern crate log;
extern crate mqtt3;
extern crate regex;
extern crate reqwest;
extern crate rumqtt;
extern crate serde_json;
extern crate threema_gateway;

mod config;
mod lpp;

use std::process::exit;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use std::thread;

use data_encoding::BASE64;
use dotenv::dotenv;
use mqtt3::Publish;
use regex::Regex;
use reqwest::{Client, StatusCode};
use rumqtt::{MqttOptions, ReconnectOptions, SecurityOptions};
use rumqtt::{MqttClient, QoS, Packet};
use serde_json::Value;
use threema_gateway::{ApiBuilder, E2eApi, RecipientKey};

use config::{Config, InfluxConfig};
use lpp::{LppDecoder, Channel, DataType};


lazy_static! {
    static ref LAST_DISTANCE: Mutex<Option<u16>> = Mutex::new(None);
    static ref LAST_VOLTAGE: Mutex<Option<f32>> = Mutex::new(None);
    static ref LAST_TEMPERATURE: Mutex<Option<f32>> = Mutex::new(None);
    static ref DATA_RATE_RE: Regex = Regex::new(r"^SF(\d+)BW(\d+)$").unwrap();
}

/// If the distance falls below this value, the system assumes that the mailbox
/// is non-empty.
static THRESHOLD: u16 = 300;

fn on_message(msg: Publish, threema_api: Arc<E2eApi>, conf: Arc<Config>) {
    debug!("Received publish packet");
    trace!("Packet: {:?}", msg);

    let decoded: Value = serde_json::from_slice(&msg.payload).unwrap();
    debug!("Payload: {:?}", decoded);

    let port = decoded.get("port")
        .expect("Uplink does not contain \"port\" field!")
        .as_u64()
        .expect("The \"port\" field does not contain a number!");
    let counter = decoded.get("counter")
        .expect("Uplink does not contain \"counter\" field!")
        .as_u64()
        .expect("The \"counter\" field does not contain a number!");
    let payload_raw = decoded.get("payload_raw")
        .expect("Uplink does not contain \"payload_raw\" field!")
        .as_str()
        .expect("The \"payload_raw\" field does not contain a string!");
    let payload_bytes = BASE64.decode(payload_raw.as_bytes())
        .expect("Raw payload is not valid Base64!");
    let deveui = decoded.get("hardware_serial")
        .expect("Uplink does not contain \"hardware_serial\" field!")
        .as_str()
        .expect("The \"hardware_serial\" field does not contain a string!");
    let airtime = decoded.get("metadata")
        .expect("Uplink does not contain \"metadata\" field!")
        .get("airtime")
        .expect("The \"metadata\" object does not contain \"airtime\" field!")
        .as_u64()
        .expect("The \"metadata.airtime\" field does not contain a number!");
    let data_rate = decoded.get("metadata")
        .expect("Uplink does not contain \"metadata\" field!")
        .get("data_rate")
        .expect("The \"metadata\" object does not contain \"data_rate\" field!")
        .as_str()
        .expect("The \"metadata.data_rate\" field does not contain a string!");
    let data_rate_captures = DATA_RATE_RE.captures(&data_rate)
        .expect("Could not parse \"data_rate\" field");
    let sf: Option<u8> = data_rate_captures.get(1).and_then(|mtch| mtch.as_str().parse().ok());
    let bw: Option<u8> = data_rate_captures.get(2).and_then(|mtch| mtch.as_str().parse().ok());

    // Log to InfluxDB
    if let Some(ref influxdb) = conf.influxdb {
        let tags = Some(format!("deveui={},port={}", deveui, port));
        send_to_influxdb(influxdb, "counter", tags.clone(), counter as f32);
        send_to_influxdb(influxdb, "airtime", tags.clone(), airtime as f32);
        if let Some(val) = sf {
            send_to_influxdb(influxdb, "sf", tags.clone(), val as f32);
        }
        if let Some(val) = bw {
            send_to_influxdb(influxdb, "bw", tags.clone(), val as f32);
        }
    };

    // Process depending on port
    match port {
        101 => process_keepalive(&payload_bytes, &deveui, threema_api, conf),
        102 => process_distance(&payload_bytes, &deveui, threema_api, conf),
        p => info!("Received message on unknown port: {}", p),
    };
}

fn process_distance(bytes: &[u8], deveui: &str, threema_api: Arc<E2eApi>, conf: Arc<Config>) {
    info!("Received distance measurement");

    // Create decoder
    let decoder = LppDecoder::new(bytes.iter());

    // Search for distance measurement
    let distance_mm = match decoder.filter_map(|m| match (m.channel, m.value) {
        (Channel::DistanceSensor, DataType::Distance(dist)) => Some(dist),
        _ => None,
    }).next() {
        Some(dist) => dist,
        None => return,
    };
    debug!("Distance is {}mm", distance_mm);

    // Compare to previous measurement
    match LAST_DISTANCE.lock() {
        Ok(mut guard) => {
            if let Some(prev_dist) = *guard {
                debug!("Previous distance was {}mm", prev_dist);
                if prev_dist < THRESHOLD && distance_mm >= THRESHOLD {
                    notify_empty(distance_mm, prev_dist, threema_api, conf.clone());
                } else if prev_dist >= THRESHOLD && distance_mm < THRESHOLD {
                    notify_full(distance_mm, prev_dist, threema_api, conf.clone());
                };
            } else {
                debug!("No previous distance stored");
            };
            *guard = Some(distance_mm);
        },
        Err(e) => error!("Could not lock LAST_DISTANCE mutex: {}", e),
    };

    // Log to InfluxDB
    if let Some(ref influxdb) = conf.influxdb {
        let tags = Some(format!("deveui={}", deveui));
        send_to_influxdb(influxdb, "distance", tags, distance_mm.into());
    };
}

fn process_keepalive(bytes: &[u8], deveui: &str, _threema_api: Arc<E2eApi>, conf: Arc<Config>) {
    info!("Received keepalive message");
    let decoder = LppDecoder::new(bytes.iter());
    let tags = Some(format!("deveui={}", deveui));
    for item in decoder {
        println!("{:?}", item);
        match (item.channel, item.value) {
            (Channel::DistanceSensor, DataType::Temperature(degrees)) => {
                match LAST_TEMPERATURE.lock() {
                    Ok(mut guard) => *guard = Some(degrees),
                    Err(e) => error!("Could not lock LAST_TEMPERATURE mutex: {}", e),
                };

                // Log to InfluxDB
                if let Some(ref influxdb) = conf.influxdb {
                    send_to_influxdb(influxdb, "temperature", tags.clone(), degrees);
                };
            },
            (Channel::Adc, DataType::AnalogInput(voltage)) => {
                match LAST_VOLTAGE.lock() {
                    Ok(mut guard) => *guard = Some(voltage),
                    Err(e) => error!("Could not lock LAST_VOLTAGE mutex: {}", e),
                };

                // Log to InfluxDB
                if let Some(ref influxdb) = conf.influxdb {
                    send_to_influxdb(influxdb, "voltage", tags.clone(), voltage);
                };
            },
            _ => {},
        }
    }

}

fn notify_full(dist: u16, prev_dist: u16, threema_api: Arc<E2eApi>, conf: Arc<Config>) {
    println!("Mailbox is full! Distance changed from {}cm to {}cm", prev_dist / 10, dist / 10);

    let mut msg = format!("\u{1F4EC} Mailbox is full! Distance changed from {:.1}cm to {:.1}cm.", (prev_dist as f32) / 10.0, (dist as f32) / 10.0);
    maybe_append_stats(&mut msg);

    for recipient in conf.threema_to.iter() {
        threema_send(&recipient, &msg, &threema_api);
    }
}

fn notify_empty(dist: u16, prev_dist: u16, threema_api: Arc<E2eApi>, conf: Arc<Config>) {
    println!("Mailbox was emptied. Distance changed from {}cm to {}cm", prev_dist / 10, dist / 10);

    let mut msg = format!("\u{1F4ED} Mailbox was emptied. Distance changed from {:.1}cm to {:.1}cm.", (prev_dist as f32) / 10.0, (dist as f32) / 10.0);
    maybe_append_stats(&mut msg);

    for recipient in conf.threema_to.iter() {
        threema_send(&recipient, &msg, &threema_api);
    }
}

fn maybe_append_stats(msg: &mut String) {
    if let (Ok(voltage_guard), Ok(temperature_guard)) = (LAST_VOLTAGE.lock(), LAST_TEMPERATURE.lock()) {
        if let (Some(voltage), Some(temperature)) = (*voltage_guard, *temperature_guard) {
            msg.push_str(&format!(" (_Voltage: {}V, temperature: {}°C._)", voltage, temperature));
        };
    };
}

fn threema_send(to: &str, msg: &str, threema_api: &Arc<E2eApi>) {
    let public_key = match threema_api.lookup_pubkey(&to) {
        Ok(pk) => pk,
        Err(e) => {
            error!("Could not look up public key for {}: {}", to, e);
            return;
        },
    };
    let recipient_key = match RecipientKey::from_str(&public_key) {
        Ok(rk) => rk,
        Err(e) => {
            error!("Could not process public key for {}: {}", to, e);
            return;
        },
    };
    let encrypted = threema_api.encrypt_text_msg(&msg, &recipient_key);
    match threema_api.send(&to, &encrypted) {
        Ok(msg_id) => debug!("Sent Threema message to {} ({})", to, msg_id),
        Err(e) => error!("Could not send message to {}: {}", to, e),
    };
}

fn send_to_influxdb(conf: &InfluxConfig, measurement: &str, tags: Option<String>, value: f32) {
    debug!("Sending {} to InfluxDB...", measurement);
    let client = match Client::new() {
        Ok(client) => client,
        Err(e) => {
            warn!("Could not create reqwest::Client instance: {}", e);
            return;
        },
    };
    let mut builder = match client.post(&format!("{}/write?db={}", conf.url, conf.db)) {
        Ok(builder) => builder,
        Err(e) => {
            warn!("Could not create reqwest::RequestBuilder instance: {}", e);
            return;
        }
    };
    let res = builder
        .body(match tags {
            Some(tags) => format!("{},{} value={}", measurement, tags, value),
            None => format!("{} value={}", measurement, value),
        })
        .basic_auth(conf.user.clone(), Some(conf.pass.clone()))
        .send();
    match res.map(|response| response.status()) {
        Ok(status) if status == StatusCode::NoContent => {
            debug!("Sent {} to InfluxDB (db={})", measurement, conf.db);
        }
        Ok(status) => {
            warn!("Unexpected status when writing {} to InfluxDB: {}", measurement, status);
        }
        Err(e) => {
            warn!("Error when writing {} to InfluxDB: {}", measurement, e);
        }
    }
}

fn main() {
    env_logger::init();
    dotenv().ok();

    println!("                  ____.----.");
    println!("        ____.----'          \\");
    println!("        \\                    \\");
    println!("         \\                    \\");
    println!("          \\                    \\");
    println!("           \\          ____.----'`--.__");
    println!("            \\___.----'          |     `--.____");
    println!("           /`-._                |       __.-' \\");
    println!("          /     `-._            ___.---'       \\");
    println!("         /          `-.____.---'                \\");
    println!("        /            / | \\                       \\");
    println!("       /            /  |  \\                   _.--'");
    println!("       `-.         /   |   \\            __.--'");
    println!("          `-._    /    |    \\     __.--'     |");
    println!("            | `-./     |     \\_.-'           |");
    println!("            |          |                     |");
    println!("            |          |                     |");
    println!("            |          |                     |");
    println!("            |          |                     |");
    println!("            |          |                     |   VK");
    println!("            |          |                     |");
    println!("     _______|          |                     |_______________");
    println!("            `-.        |                  _.-'");
    println!("               `-.     |           __..--'");
    println!("                  `-.  |      __.-'");
    println!("                     `-|__.--'");
    println!("");
    println!("Welcome to smartmail!");
    println!("");

    // Load configuration
    let conf = Arc::new(
        match Config::init() {
            Ok(conf) => conf,
            Err(msg) => {
                println!("Error: {}", msg);
                exit(1);
            },
        }
    );

    // Set up Threema Gateway API
    let api = Arc::new(
        ApiBuilder::new(conf.threema_from.as_ref(), conf.threema_secret.as_ref())
            .with_private_key_str(&conf.threema_private_key)
            .and_then(|builder| builder.into_e2e())
            .unwrap_or_else(|e| {
                println!("Could not initialize Threema E2E API: {}", e);
                exit(2);
            })
    );

    // Set up MQTT connection
    let client_id = format!("smartmail-{}", {
        let start = SystemTime::now();
        let since_the_epoch = start.duration_since(UNIX_EPOCH).expect("Time went backwards");
        since_the_epoch.as_secs()
    });
    let client_options = MqttOptions::new(client_id, "eu.thethings.network:1883".to_string())
            .unwrap_or_else(|e| {
                println!("Could not initialize MqttOptions: {}", e);
                exit(3);
            })
            .set_keep_alive(60)
            .set_reconnect_opts(ReconnectOptions::Always(3))
            .set_security_opts(SecurityOptions::UsernamePassword((
                conf.ttn_app_id.clone(),
                conf.ttn_access_key.clone(),
            )));

    println!("--> Connecting to the Things Network...");
    let (mut client, receiver) = MqttClient::start(client_options);

    println!("--> Subscribing to uplink messages...");
    let topics = vec![
        ("+/devices/+/activations", QoS::AtMostOnce),
        ("+/devices/+/up", QoS::AtMostOnce),
    ];
    client.subscribe(topics).expect("Subcription failure");

    thread::spawn(move || {
        println!("--> Listening!");
        for packet in receiver {
            if let Packet::Publish(publish) = packet {
                on_message(publish, api.clone(), conf.clone());
            } else {
                debug!("Received non-publish packet: {:?}", packet);
            }
        }
    }).join().unwrap();
}
