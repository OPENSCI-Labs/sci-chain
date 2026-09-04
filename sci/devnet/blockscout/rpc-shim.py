#!/usr/bin/env python3
"""JSON-RPC shim for Blockscout: presents SCI Chain AA txs (type 0x76) as EIP-1559
(type 0x2) so stock Blockscout v7.0.2 can index them.

Blockscout's EthereumJSONRPC.Transaction.do_elixir_to_params/1 matches on the
presence of the standard keys gas/input/value (+ type). SCI's AA tx RPC shape uses
`gasLimit` (not `gas`) and nests to/value/input inside calls[0], so no clause
matches -> FunctionClauseError -> block import crash-loops. This shim rewrites every
type:"0x76" object in the upstream response: type->0x2 and lifts
gas/input/value/to from gasLimit + calls[0]. First-call approximation for multi-call
batches (consistent with the rest of the Plan A stack, which maps AA receipts to
EIP-1559 and surfaces the first call in TransactionRequest).
"""
import os
import json
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

UPSTREAM = os.environ.get("UPSTREAM", "http://host.docker.internal:8545")
LISTEN_PORT = int(os.environ.get("LISTEN_PORT", "8545"))


def patch(obj):
    """Recursively rewrite AA (type 0x76) tx objects to EIP-1559 (type 0x2) shape."""
    if isinstance(obj, dict):
        if obj.get("type") == "0x76":
            calls = obj.get("calls") or []
            first = calls[0] if calls else {}
            if "gasLimit" in obj:
                obj["gas"] = obj["gasLimit"]
            obj["input"] = first.get("input", "0x")
            obj["value"] = first.get("value", "0x0")
            obj["to"] = first.get("to")
            obj["type"] = "0x2"
        for v in list(obj.values()):
            patch(v)
    elif isinstance(obj, list):
        for v in obj:
            patch(v)
    return obj


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *_):
        pass

    def _forward(self, body):
        req = urllib.request.Request(
            UPSTREAM, data=body, method="POST",
            headers={"Content-Type": "application/json", "Accept-Encoding": "identity"},
        )
        with urllib.request.urlopen(req, timeout=30) as resp:
            return resp.read()

    def do_POST(self):
        length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(length)
        try:
            raw = self._forward(body)
        except Exception as e:  # upstream error -> 502
            msg = json.dumps({"error": str(e)}).encode()
            self.send_response(502)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(msg)))
            self.end_headers()
            self.wfile.write(msg)
            return
        try:
            out = json.dumps(patch(json.loads(raw))).encode()
        except Exception:
            out = raw  # not JSON / parse failure -> pass through untouched
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(out)))
        self.end_headers()
        self.wfile.write(out)

    def do_GET(self):  # health
        self.send_response(200)
        self.send_header("Content-Length", "2")
        self.end_headers()
        self.wfile.write(b"ok")


if __name__ == "__main__":
    ThreadingHTTPServer(("0.0.0.0", LISTEN_PORT), Handler).serve_forever()
