#!/usr/bin/env python3
"""Serve echo_byte's Web Bluetooth client from a trusted HTTPS origin."""

from __future__ import annotations

import argparse
import functools
import http.server
import ssl
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_CERT_DIR = Path.home() / "scripts" / "cert"


class BlePageHandler(http.server.SimpleHTTPRequestHandler):
    def do_GET(self) -> None:  # noqa: N802 - stdlib callback name
        if self.path in ("", "/"):
            self.send_response(302)
            self.send_header("Location", "/ble_provision.html")
            self.end_headers()
            return
        super().do_GET()

    def end_headers(self) -> None:
        self.send_header("Cache-Control", "no-store")
        self.send_header("X-Content-Type-Options", "nosniff")
        self.send_header("Referrer-Policy", "no-referrer")
        super().end_headers()


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default="0.0.0.0", help="listen address")
    parser.add_argument("--port", type=int, default=8443, help="HTTPS port")
    parser.add_argument(
        "--directory",
        type=Path,
        default=PROJECT_ROOT / "web",
        help="directory containing ble_provision.html",
    )
    parser.add_argument(
        "--cert",
        type=Path,
        default=DEFAULT_CERT_DIR / "192.168.3.2+3.pem",
        help="TLS certificate chain",
    )
    parser.add_argument(
        "--key",
        type=Path,
        default=DEFAULT_CERT_DIR / "192.168.3.2+3-key.pem",
        help="TLS private key",
    )
    return parser.parse_args()


def require_file(path: Path, description: str) -> Path:
    path = path.expanduser().resolve()
    if not path.is_file():
        raise SystemExit(f"{description} not found: {path}")
    return path


def main() -> None:
    args = parse_args()
    directory = require_file(args.directory / "ble_provision.html", "BLE page").parent
    certificate = require_file(args.cert, "TLS certificate")
    private_key = require_file(args.key, "TLS private key")

    handler = functools.partial(BlePageHandler, directory=str(directory))
    server = http.server.ThreadingHTTPServer((args.host, args.port), handler)
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.minimum_version = ssl.TLSVersion.TLSv1_2
    context.load_cert_chain(certfile=certificate, keyfile=private_key)
    server.socket = context.wrap_socket(server.socket, server_side=True)

    print(f"Serving {directory / 'ble_provision.html'}", flush=True)
    print(f"Open https://192.168.3.2:{args.port}/ on the phone", flush=True)
    print("Press Ctrl+C to stop", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
