//! Partial implementation of a Cayenne LPP format decoder.

use std::convert::From;
use std::iter::Iterator;
use std::slice::Iter;

use byteorder::{ByteOrder, BigEndian};


/// The LPP channels used in the ax-sense.
#[derive(Debug, PartialEq, Copy, Clone)]
pub enum Channel {
    DistanceSensor,
    Adc,
    Other(u8),
}

impl From<u8> for Channel {
    fn from(val: u8) -> Self {
        match val {
            1 => Channel::DistanceSensor,
            4 => Channel::Adc,
            c => Channel::Other(c),
        }
    }
}

/// The LPP data types used in the ax-sense.
/// 
/// The types wrap their values.
#[derive(Debug, PartialEq, Copy, Clone)]
pub enum DataType {
    AnalogInput(f32),
    Temperature(f32),
    Distance(u16),
}

#[derive(Debug, PartialEq, Clone)]
pub struct Measurement {
    pub channel: Channel,
    pub value: DataType,
}

impl Measurement {
    pub fn new(channel: Channel, value: DataType) -> Self {
        Measurement {
            channel: channel,
            value: value,
        }
    }
}

#[derive(Debug)]
pub struct LppDecoder<'a> {
   bytes: Iter<'a, u8>, 
}

impl<'a> LppDecoder<'a> {
    pub fn new(bytes: Iter<'a, u8>) -> Self {
        LppDecoder {
            bytes: bytes,
        }
    }
}

impl<'a> Iterator for LppDecoder<'a> {
    type Item = Measurement;

    /// Return the next measurement from this packet.
    /// 
    /// Note that errors are simply ignored and logged with WARN level.
    fn next(&mut self) -> Option<Self::Item> {
        let channel = match self.bytes.next() {
            Some(channel_id) => Channel::from(*channel_id),
            None => return None,
        };
        let value = match self.bytes.next() {
            // Parse analog input values
            Some(&0x02) => {
                if let (Some(hi), Some(lo)) = (self.bytes.next(), self.bytes.next()) {
                    DataType::AnalogInput(BigEndian::read_i16(&[*hi, *lo]) as f32 / 100.0)
                } else {
                    warn!("Received incomplete analog input data from channel {:?}", channel);
                    return None;
                }
            },

            // Parse temperature values
            Some(&0x67) => {
                if let (Some(hi), Some(lo)) = (self.bytes.next(), self.bytes.next()) {
                    DataType::Temperature(BigEndian::read_i16(&[*hi, *lo]) as f32 / 10.0)
                } else {
                    warn!("Received incomplete temperature data from channel {:?}", channel);
                    return None;
                }
            },

            // Parse distance values
            Some(&0x82) => {
                if let (Some(hi), Some(lo)) = (self.bytes.next(), self.bytes.next()) {
                    DataType::Distance((*hi as u16 * 256) + *lo as u16)
                } else {
                    warn!("Received incomplete distance data from channel {:?}", channel);
                    return None;
                }
            },

            // Unknown messages
            Some(t) => {
                warn!("Received data from channel {:?} with unknown data type: {}", channel, t);
                return None
            },

            // Incomplete messages
            None => {
                warn!("Received incomplete data from channel {:?}", channel);
                return None
            },
        };
        Some(Measurement::new(channel, value))
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_distance_data() {
        let data = [0x01, 0x82, 0x01, 0x3D];
        let mut decoder = LppDecoder::new(data.iter());
        assert_eq!(
            decoder.next().unwrap(),
            Measurement::new(Channel::DistanceSensor, DataType::Distance(317))
        );
    }

    #[test]
    fn test_keepalive_data() {
        let data = [
            0x01, 0x67, 0x00, 0xE6,
            0x04, 0x02, 0x01, 0x7A,
        ];
        let mut decoder = LppDecoder::new(data.iter());
        assert_eq!(
            decoder.next().unwrap(),
            Measurement::new(Channel::DistanceSensor, DataType::Temperature(23.0))
        );
        assert_eq!(
            decoder.next().unwrap(),
            Measurement::new(Channel::Adc, DataType::AnalogInput(3.78))
        );
    }

}
