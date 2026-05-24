"""Simple HTTP server in Python."""
from __future__ import annotations
import json
from http.server import BaseHTTPRequestHandler, HTTPServer
from typing import Any


class RequestHandler(BaseHTTPRequestHandler):
    """Handle incoming HTTP requests."""

    def do_GET(self) -> None:
        if self.path == "/health":
            self._send_json({"status": "ok"})
        elif self.path == "/users":
            self._send_json({"users": []})
        else:
            self._send_error(404, "Not found")

    def do_POST(self) -> None:
        content_length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(content_length)
        try:
            data = json.loads(body)
        except json.JSONDecodeError:
            self._send_error(400, "Invalid JSON")
            return
        self._send_json({"received": data}, status=201)

    def _send_json(self, data: Any, status: int = 200) -> None:
        payload = json.dumps(data).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def _send_error(self, status: int, message: str) -> None:
        self._send_json({"error": message}, status=status)

    def log_message(self, fmt: str, *args: Any) -> None:
        pass  # suppress default access log


def create_server(host: str = "localhost", port: int = 8080) -> HTTPServer:
    return HTTPServer((host, port), RequestHandler)


if __name__ == "__main__":
    srv = create_server()
    print(f"Listening on http://localhost:8080")
    srv.serve_forever()
