extern crate data_encoding;
extern crate rumqtt;
extern crate serde_json;

use std::thread;
use std::time::Duration;

use data_encoding::BASE64;
use rumqtt::{MqttOptions, MqttClient, QoS};
use rumqtt::{MqttCallback, Message};
use serde_json::{Value};

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
}

fn main() {
    println!("Hello, smartmail!");

    let client_options = MqttOptions::new()
            .set_keep_alive(5)
            .set_reconnect(3)
            .set_client_id("smartmail")
            .set_user_name(XXX)
            .set_password(YYY)
            .set_broker("eu.thethings.network:1883");

    let callbacks = MqttCallback::new().on_message(on_message);

    let mut request = MqttClient::start(client_options, Some(callbacks)).expect("Coudn't start");

    let topics = vec![
        ("+/devices/+/activations", QoS::Level2),
        ("+/devices/+/up", QoS::Level2),
    ];
    request.subscribe(topics).expect("Subcription failure");

    loop {
        thread::sleep(Duration::from_secs(10));
    }
}
