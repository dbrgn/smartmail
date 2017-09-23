extern crate byteorder;
extern crate data_encoding;
extern crate env_logger;
#[macro_use] extern crate lazy_static;
#[macro_use] extern crate log;
extern crate rumqtt;
extern crate serde_json;

mod lpp;

use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use data_encoding::BASE64;
use rumqtt::{MqttOptions, MqttClient, QoS};
use rumqtt::{MqttCallback, Message};
use serde_json::{Value};

use lpp::{LppDecoder, Channel, DataType};


lazy_static! {
    static ref LAST_DISTANCE: Mutex<Option<u16>> = Mutex::new(None);
}

/// If the distance falls below this value, the system assumes that the mailbox
/// is non-empty.
static THRESHOLD: u16 = 300;

fn on_message(msg: Message) {
    println!("Received payload: {:?}", msg);

    let decoded: Value = serde_json::from_slice(&msg.payload).unwrap();
    println!("Payload: {:?}", decoded);

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

    println!("Msg on port {}: {:?}", port, payload_bytes);
    match port {
        1 => process_distance(&payload_bytes),
        101 => process_keepalive(&payload_bytes),
        102 => process_distance(&payload_bytes),
        p => info!("Received message on unknown port: {}", p),
    };
}

fn process_distance(bytes: &[u8]) {
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
                    notify_empty(distance_mm, prev_dist);
                } else if prev_dist >= THRESHOLD && distance_mm < THRESHOLD {
                    notify_full(distance_mm, prev_dist);
                };
            } else {
                debug!("No previous distance stored");
            };
            *guard = Some(distance_mm);
        },
        Err(e) => error!("Could not lock LAST_DISTANCE mutex: {}", e),
    };
}

fn process_keepalive(bytes: &[u8]) {
    info!("Received keepalive message");
    let decoder = LppDecoder::new(bytes.iter());
    for item in decoder {
        println!("{:?}", item);
    }
}

fn notify_full(dist: u16, prev_dist: u16) {
    println!("Mailbox is full! Distance changed from {}cm to {}cm", prev_dist / 10, dist / 10);
}

fn notify_empty(dist: u16, prev_dist: u16) {
    println!("Mailbox was emptied. Distance changed from {}cm to {}cm", prev_dist / 10, dist / 10);
}

fn main() {
    env_logger::init().unwrap();

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

    let client_options = MqttOptions::new()
            .set_keep_alive(5)
            .set_reconnect(3)
            .set_client_id("smartmail")
            .set_user_name(XXX)
            .set_password(YYY)
            .set_broker("eu.thethings.network:1883");

    let callbacks = MqttCallback::new().on_message(on_message);

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
