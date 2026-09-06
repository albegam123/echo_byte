use std::io::ErrorKind;
use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{Context, Result};

use crate::config::AP_IP;

const DNS_PORT: u16 = 53;
const MAX_DNS_PACKET: usize = 512;
const DNS_TYPE_A: u16 = 1;
const DNS_TYPE_ANY: u16 = 255;
const DNS_CLASS_IN: u16 = 1;

pub struct DnsServer {
    stopping: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl DnsServer {
    pub fn start() -> Result<Self> {
        let socket = UdpSocket::bind(("0.0.0.0", DNS_PORT)).context("bind captive DNS UDP/53")?;
        socket
            .set_read_timeout(Some(Duration::from_millis(250)))
            .context("set captive DNS timeout")?;

        let stopping = Arc::new(AtomicBool::new(false));
        let worker_stopping = stopping.clone();
        let worker = thread::Builder::new()
            .name("echo-dns".into())
            .stack_size(4096)
            .spawn(move || dns_loop(socket, &worker_stopping))
            .context("spawn captive DNS task")?;

        log::info!("captive DNS listening on UDP/53 -> {AP_IP}");
        Ok(Self {
            stopping,
            worker: Some(worker),
        })
    }
}

impl Drop for DnsServer {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            if worker.join().is_err() {
                log::warn!("captive DNS task panicked while stopping");
            }
        }
        log::info!("captive DNS stopped");
    }
}

fn dns_loop(socket: UdpSocket, stopping: &AtomicBool) {
    let mut request = [0_u8; MAX_DNS_PACKET];
    let mut response = [0_u8; MAX_DNS_PACKET];

    while !stopping.load(Ordering::Acquire) {
        match socket.recv_from(&mut request) {
            Ok((length, peer)) => {
                if let Some(response_length) = build_response(&request[..length], &mut response) {
                    if let Err(error) = socket.send_to(&response[..response_length], peer) {
                        log::warn!("captive DNS response failed: {error}");
                    }
                }
            }
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(error) => log::warn!("captive DNS receive failed: {error}"),
        }
    }
}

/// Builds a minimal authoritative-looking response containing one IPv4 A
/// record. Malformed and already-response packets are ignored.
fn build_response(request: &[u8], response: &mut [u8; MAX_DNS_PACKET]) -> Option<usize> {
    if request.len() < 12
        || request.len() > MAX_DNS_PACKET
        || request[2] & 0x80 != 0
        || request[2] & 0x78 != 0
    {
        return None;
    }
    let questions = u16::from_be_bytes([request[4], request[5]]);
    if questions == 0 {
        return None;
    }

    let name_end = question_name_end(request, 12)?;
    let question_end = name_end.checked_add(4)?;
    if question_end > request.len() {
        return None;
    }

    let query_type = u16::from_be_bytes([request[name_end], request[name_end + 1]]);
    let query_class = u16::from_be_bytes([request[name_end + 2], request[name_end + 3]]);
    let include_ipv4 =
        query_class == DNS_CLASS_IN && matches!(query_type, DNS_TYPE_A | DNS_TYPE_ANY);
    let response_length = question_end + if include_ipv4 { 16 } else { 0 };
    if response_length > response.len() {
        return None;
    }

    response[..question_end].copy_from_slice(&request[..question_end]);
    // QR=1, AA=1, copy RD, RA=1, RCODE=0.
    let recursion_desired = request[2] & 0x01;
    response[2] = 0x84 | recursion_desired;
    response[3] = 0x80;
    response[4..6].copy_from_slice(&1_u16.to_be_bytes());
    response[6..8].copy_from_slice(&u16::from(include_ipv4).to_be_bytes());
    response[8..12].fill(0);

    if !include_ipv4 {
        // An IPv4-only captive portal has no useful AAAA response. Returning
        // NOERROR/NODATA lets the client immediately fall back to its A query.
        return Some(question_end);
    }

    let answer = &mut response[question_end..question_end + 16];
    answer[0..2].copy_from_slice(&[0xc0, 0x0c]); // compressed QNAME
    answer[2..4].copy_from_slice(&1_u16.to_be_bytes()); // A
    answer[4..6].copy_from_slice(&1_u16.to_be_bytes()); // IN
    answer[6..10].copy_from_slice(&30_u32.to_be_bytes());
    answer[10..12].copy_from_slice(&4_u16.to_be_bytes());
    answer[12..16].copy_from_slice(&AP_IP.octets());

    Some(response_length)
}

fn question_name_end(packet: &[u8], mut cursor: usize) -> Option<usize> {
    let mut labels = 0;
    loop {
        let length = *packet.get(cursor)? as usize;
        cursor += 1;
        if length == 0 {
            return Some(cursor);
        }
        // Compression in a question is legal but deliberately rejected here;
        // captive-portal probes use ordinary labels and rejecting pointers
        // keeps parsing bounded and loop-free.
        if length & 0xc0 != 0 || length > 63 {
            return None;
        }
        cursor = cursor.checked_add(length)?;
        if cursor > packet.len() {
            return None;
        }
        labels += 1;
        if labels > 32 {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_answer_points_at_portal() {
        let request = [
            0x12, 0x34, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x07, b'e',
            b'x', b'a', b'm', b'p', b'l', b'e', 0x03, b'c', b'o', b'm', 0x00, 0x00, 0x01, 0x00,
            0x01,
        ];
        let mut response = [0_u8; MAX_DNS_PACKET];
        let length = build_response(&request, &mut response).unwrap();

        assert_eq!(&response[0..2], &[0x12, 0x34]);
        assert_eq!(&response[6..8], &[0, 1]);
        assert_eq!(&response[length - 4..length], &AP_IP.octets());
    }

    #[test]
    fn ipv6_query_returns_no_data_instead_of_wrong_record_type() {
        let request = [
            0x12, 0x34, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04, b't',
            b'e', b's', b't', 0x00, 0x00, 0x1c, 0x00, 0x01,
        ];
        let mut response = [0_u8; MAX_DNS_PACKET];
        let length = build_response(&request, &mut response).unwrap();

        assert_eq!(length, request.len());
        assert_eq!(&response[6..8], &[0, 0]);
    }
}
