#!/usr/bin/env python3
"""Serve a fake GitHub releases response for manual update-check testing."""

import argparse
import json
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlsplit


def release_payload(version: str) -> bytes:
    release = {
        "tag_name": version,
        "prerelease": False,
        "assets": [
            {
                "name": "nvim-gpui-test",
                "browser_download_url": "https://example.com/nvim-gpui-test",
                "digest": None,
            }
        ],
        "tarball_url": "https://example.com/nvim-gpui-tarball",
        "zipball_url": "https://example.com/nvim-gpui-zip",
    }
    return json.dumps([release]).encode("utf-8")


def make_handler(path: str, body: bytes):
    class MockUpdateHandler(BaseHTTPRequestHandler):
        def do_GET(self) -> None:  # noqa: N802 - required by BaseHTTPRequestHandler
            request_path = urlsplit(self.path).path
            if request_path != path:
                self.send_error(404, f"expected {path}")
                return

            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Connection", "close")
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, format: str, *args: object) -> None:
            print(f"[{self.log_date_time_string()}] {format % args}", file=sys.stderr)

    return MockUpdateHandler


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8787)
    parser.add_argument("--path", default="/fake/releases")
    parser.add_argument("--version", default="v0.7.3")
    args = parser.parse_args()

    path = "/" + args.path.lstrip("/")
    body = release_payload(args.version)
    server = ThreadingHTTPServer((args.host, args.port), make_handler(path, body))
    print(
        f"mock update server listening on http://{args.host}:{args.port}{path} "
        f"(version {args.version})",
        flush=True,
    )
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print("\nmock update server stopped", file=sys.stderr)
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
