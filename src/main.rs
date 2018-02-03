extern crate byteorder;
extern crate data_encoding;
extern crate dotenv;
extern crate env_logger;
#[macro_use] extern crate lazy_static;
#[macro_use] extern crate log;
extern crate rumqtt;
extern crate serde_json;
extern crate threema_gateway;

mod config;
mod lpp;

use std::process::exit;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use data_encoding::BASE64;
use dotenv::dotenv;
use rumqtt::{MqttOptions, MqttClient, QoS};
use rumqtt::{MqttCallback, Message};
use serde_json::Value;
use threema_gateway::{ApiBuilder, E2eApi, RecipientKey};

use config::Config;
use lpp::{LppDecoder, Channel, DataType};


lazy_static! {
    static ref LAST_DISTANCE: Mutex<Option<u16>> = Mutex::new(None);
    static ref LAST_VOLTAGE: Mutex<Option<f32>> = Mutex::new(None);
    static ref LAST_TEMPERATURE: Mutex<Option<f32>> = Mutex::new(None);
}

/// If the distance falls below this value, the system assumes that the mailbox
/// is non-empty.
static THRESHOLD: u16 = 300;

fn on_message(msg: Message, threema_api: Arc<E2eApi>, conf: Arc<Config>) {
    debug!("Received payload: {:?}", msg);

    let decoded: Value = serde_json::from_slice(&msg.payload).unwrap();
    debug!("Payload: {:?}", decoded);

    let port = decoded.get("port")
        .expect("Uplink does not contain \"port\" field!")
        .as_u64()
        .expect("The \"port\" field does not contain a number!");
    let payload_raw = decoded.get("payload_raw")
        .expect("Uplink does not contain \"payload_raw\" field!")
        .as_str()
        .expect("The \"payload_raw\" field does not contain a string!");
    let payload_bytes = BASE64.decode(payload_raw.as_bytes())
        .expect("Raw payload is not valid Base64!");

    match port {
        101 => process_keepalive(&payload_bytes, threema_api, conf),
        102 => process_distance(&payload_bytes, threema_api, conf),
        p => info!("Received message on unknown port: {}", p),
    };
}

fn process_distance(bytes: &[u8], threema_api: Arc<E2eApi>, conf: Arc<Config>) {
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
                    notify_empty(distance_mm, prev_dist, threema_api, conf);
                } else if prev_dist >= THRESHOLD && distance_mm < THRESHOLD {
                    notify_full(distance_mm, prev_dist, threema_api, conf);
                };
            } else {
                debug!("No previous distance stored");
            };
            *guard = Some(distance_mm);
        },
        Err(e) => error!("Could not lock LAST_DISTANCE mutex: {}", e),
    };
}

fn process_keepalive(bytes: &[u8], _threema_api: Arc<E2eApi>, _conf: Arc<Config>) {
    info!("Received keepalive message");
    let decoder = LppDecoder::new(bytes.iter());
    for item in decoder {
        println!("{:?}", item);
        match (item.channel, item.value) {
            (Channel::DistanceSensor, DataType::Temperature(degrees)) => {
                match LAST_TEMPERATURE.lock() {
                    Ok(mut guard) => *guard = Some(degrees),
                    Err(e) => error!("Could not lock LAST_TEMPERATURE mutex: {}", e),
                };
            },
            (Channel::Adc, DataType::AnalogInput(voltage)) => {
                match LAST_VOLTAGE.lock() {
                    Ok(mut guard) => *guard = Some(voltage),
                    Err(e) => error!("Could not lock LAST_VOLTAGE mutex: {}", e),
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
    let client_options = MqttOptions::new()
            .set_keep_alive(5)
            .set_reconnect(3)
            .set_client_id("smartmail")
            .set_user_name(&conf.ttn_app_id)
            .set_password(&conf.ttn_access_key)
            .set_broker("eu.thethings.network:1883");
    let callbacks = MqttCallback::new().on_message(move |msg| on_message(msg, api.clone(), conf.clone()));

    println!("--> Connecting to the Things Network...");
    let mut request = MqttClient::start(client_options, Some(callbacks)).expect("Coudn't start");

    println!("--> Subscribing to uplink messages...");
    let topics = vec![
        ("+/devices/+/activations", QoS::Level2),
        ("+/devices/+/up", QoS::Level2),
    ];
    request.subscribe(topics).expect("Subcription failure");

    println!("--> Listening!");
    loop {
        thread::sleep(Duration::from_secs(10));
    }
}
