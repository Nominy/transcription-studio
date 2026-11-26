"""Exercise the built stdio helper against a loopback mock, never against Babel."""
import json
import os
from pathlib import Path
import subprocess
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

requests = []
response = [{"result": {"data": {"json": {"kind": "saved"}}}}]

class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        self.reply()

    def do_POST(self):
        self.reply()

    def reply(self):
        body = self.rfile.read(int(self.headers.get("Content-Length", "0")))
        requests.append((self.command, self.path, body))
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(json.dumps(response).encode())

    def log_message(self, *args):
        pass

root = Path(__file__).resolve().parent.parent
executable = root / "target/debug/studio-babel-service.exe"
server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
threading.Thread(target=server.serve_forever, daemon=True).start()
env = os.environ.copy()
for key in list(env):
    if key.startswith("STUDIO_BABEL_") or key.upper().endswith("_PROXY"):
        env.pop(key, None)
env.update(STUDIO_BABEL_SERVICE_BASE_URL=f"http://127.0.0.1:{server.server_port}",
           STUDIO_BABEL_SERVICE_AUTHORIZATION="Bearer mock-only",
           NO_PROXY="127.0.0.1,localhost")
try:
    for operation, method in [("evalTasks.saveProgress", "POST"),
                              ("transcriptions.getPresignedChunkUrls", "GET")]:
        envelope = {"operation": operation, "payload": {"sessionId": "mock-session", "tasks": []}}
        before = len(requests)
        result = subprocess.run([str(executable)], input=json.dumps(envelope),
                                capture_output=True, text=True, env=env, timeout=30)
        assert result.returncode == 0, result.stderr
        assert json.loads(result.stdout) == response
        assert len(requests) == before + 1, "Request must be sent exactly once"
        actual_method, path, body = requests[-1]
        assert actual_method == method and path.startswith(f"/api/trpc/{operation}?")
        if method == "POST":
            assert json.loads(body)["0"]["json"] == envelope["payload"]
        print(f"PASS: {operation} / {method} / one request / valid stdout JSON")
finally:
    server.shutdown()
    server.server_close()
