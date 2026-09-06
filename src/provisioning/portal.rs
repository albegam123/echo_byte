use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::{Context, Result};
use async_channel::Sender;
use embedded_svc::http::{Headers, Method};
use embedded_svc::io::{Read, Write};
use esp_idf_svc::http::server::{Configuration, EspHttpServer};

use super::dns::DnsServer;
use super::{networks_json, submit, Credentials};
use crate::config::{MAX_CREDENTIAL_JSON_LEN, PORTAL_URL};
use crate::wifi::ScannedNetwork;

static INDEX_HTML: &str = include_str!("portal.html");

pub struct CaptivePortal {
    // HTTP is dropped first so no callback can outlive its captured sender.
    http: Option<EspHttpServer<'static>>,
    dns: Option<DnsServer>,
}

impl CaptivePortal {
    pub fn start(
        sender: Sender<Credentials>,
        winner_chosen: Arc<AtomicBool>,
        networks: &[ScannedNetwork],
    ) -> Result<Self> {
        let dns = DnsServer::start()?;
        let networks = Arc::new(networks_json(networks)?);

        let mut http = EspHttpServer::new(&Configuration {
            stack_size: 8192,
            max_uri_handlers: 16,
            uri_match_wildcard: true,
            ..Default::default()
        })
        .context("start captive HTTP server")?;

        http.fn_handler("/", Method::Get, serve_portal)?;

        let list = networks.clone();
        http.fn_handler("/api/networks", Method::Get, move |request| {
            let mut response = request.into_response(
                200,
                Some("OK"),
                &[
                    ("Content-Type", "application/json; charset=utf-8"),
                    ("Cache-Control", "no-store"),
                ],
            )?;
            response.write_all(list.as_bytes())
        })?;

        http.fn_handler::<anyhow::Error, _>("/api/provision", Method::Post, move |mut request| {
            let Some(length) = request.content_len().map(|length| length as usize) else {
                send_json(
                    request,
                    411,
                    br#"{"ok":false,"error":"Content-Length required"}"#,
                )?;
                return Ok(());
            };
            if length == 0 || length > MAX_CREDENTIAL_JSON_LEN {
                send_json(
                    request,
                    413,
                    br#"{"ok":false,"error":"Invalid payload length"}"#,
                )?;
                return Ok(());
            }

            let mut body = [0_u8; MAX_CREDENTIAL_JSON_LEN];
            request.read_exact(&mut body[..length])?;
            match submit(&sender, &winner_chosen, &body[..length]) {
                Ok(()) => {
                    send_json(request, 202, br#"{"ok":true,"status":"connecting"}"#)?;
                    log::info!("portal provisioning request accepted");
                }
                Err(error) => {
                    log::warn!("portal provisioning request rejected: {error}");
                    let status = if error.contains("already won") {
                        409
                    } else {
                        400
                    };
                    send_json(
                        request,
                        status,
                        br#"{"ok":false,"error":"Invalid or duplicate request"}"#,
                    )?;
                }
            }
            Ok(())
        })?;

        // OS-specific connectivity probes. A redirect or a page different
        // from the expected success body prompts the OS captive portal UI.
        for path in [
            "/generate_204",
            "/gen_204",
            "/connecttest.txt",
            "/ncsi.txt",
            "/canonical.html",
            "/redirect",
        ] {
            http.fn_handler(path, Method::Get, redirect_to_portal)?;
        }
        for path in ["/hotspot-detect.html", "/library/test/success.html"] {
            http.fn_handler(path, Method::Get, serve_portal)?;
        }

        http.fn_handler("/*", Method::Get, redirect_to_portal)?;

        log::info!("captive HTTP portal listening on TCP/80");
        Ok(Self {
            http: Some(http),
            dns: Some(dns),
        })
    }
}

impl Drop for CaptivePortal {
    fn drop(&mut self) {
        drop(self.http.take());
        drop(self.dns.take());
        log::info!("captive portal HTTP/DNS services stopped");
    }
}

fn serve_portal(
    request: esp_idf_svc::http::server::Request<
        &mut esp_idf_svc::http::server::EspHttpConnection<'_>,
    >,
) -> anyhow::Result<()> {
    let mut response = request.into_response(
        200,
        Some("OK"),
        &[
            ("Content-Type", "text/html; charset=utf-8"),
            ("Cache-Control", "no-store, no-cache, must-revalidate"),
        ],
    )?;
    response.write_all(INDEX_HTML.as_bytes())?;
    Ok(())
}

fn redirect_to_portal(
    request: esp_idf_svc::http::server::Request<
        &mut esp_idf_svc::http::server::EspHttpConnection<'_>,
    >,
) -> anyhow::Result<()> {
    let mut response = request.into_response(
        302,
        Some("Found"),
        &[("Location", PORTAL_URL), ("Cache-Control", "no-store")],
    )?;
    response.write_all(b"Open the EchoByte setup portal")?;
    Ok(())
}

fn send_json(
    request: esp_idf_svc::http::server::Request<
        &mut esp_idf_svc::http::server::EspHttpConnection<'_>,
    >,
    status: u16,
    body: &[u8],
) -> anyhow::Result<()> {
    let mut response = request.into_response(
        status,
        None,
        &[
            ("Content-Type", "application/json; charset=utf-8"),
            ("Cache-Control", "no-store"),
        ],
    )?;
    response.write_all(body)?;
    Ok(())
}
