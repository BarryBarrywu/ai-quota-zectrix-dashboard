"""Exercise the real companion loop against three loopback HTTP services."""
import datetime
import http.server
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import threading
import time

driver, binary, directory = sys.argv[1:]
root = Path(directory)
base = 1_800_000_000
fixture = dict(deviceId="private-fixture-device", pageId=3)
(root / "settings.json").write_text(json.dumps(fixture))
snapshot = '{"quota":{"gemini-5h":{"remaining_fraction":0.3,"reset_time":"2020-01-01T00:00:00Z"},"gemini-weekly":{"remaining_fraction":0.8,"reset_time":"2020-01-01T00:00:00Z"}}}'
(root / "status.json").write_text(snapshot)
fake = root / "codex"
fake.write_text("""#!/usr/bin/env python3
import json, os, pathlib, subprocess, sys
root = pathlib.Path(__file__).parent
assert sys.argv[1:] == ['app-server','--stdio']
helper = subprocess.Popen(['sleep', '180'],stdout=subprocess.DEVNULL)
(root/'pids').write_text(f'{os.getpid()} {helper.pid}')
for line in sys.stdin:
    m = json.loads(line)
    if m['method'] == 'initialized': continue
    if m['method'] == 'initialize': result = {}
    else:
        assert m['method'] == 'account/rateLimits/read'
        with (root/'reads').open('a') as f: print('read',file=f)
        result = {'private':'private-codex-payload'} if (root/'bad-codex').exists() else {
            'rateLimits': {'primary':{'usedPercent':32,'resetsAt':1800000059+10,'windowDurationMins':10080}}}
    print(json.dumps({'result':result,'id':m['id']}),flush=True)
""")
fake.chmod(0o755)


def iso(stamp):
    return datetime.datetime.fromtimestamp(stamp, datetime.timezone.utc).isoformat()


class Service(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_GET(self):
        assert self.path == "/openapi/v1/status"
        self.server.visits.append(time.monotonic())
        if self.server.fail:
            self.send_response(503)
            payload = b"private-radar-payload"
        elif self.headers.get("If-None-Match") == '"v1"':
            self.send_response(304)
            payload = b""
        else:
            self.send_response(200)
            self.send_header("ETag", '"v1"')
            payload = json.dumps({"ok": True, "data": {"latestGlobal": None, "latestBanked": None, "nextScheduled": {
                "kind": "reset_scheduled", "scheduleState": "pending",
                "scheduleWindow": {"startAt": iso(base + 9_000), "endAt": iso(base + 90_000)}
            }}}).encode()
        self.end_headers()
        self.wfile.write(payload)

    def do_POST(self):
        assert self.path == "/open/v1/devices/private-fixture-device/display/image"
        assert self.headers["X-API-Key"] == "fixture-key"
        body = self.rfile.read(int(self.headers["Content-Length"]))
        assert b"image/png" in body and b"\x89PNG" in body
        self.server.visits.append(json.loads((root / "publisher.json").read_text())["next_attempt"] - 60)
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b'{"code":1,"message":"private-cloud-payload"}' if self.server.fail else b'{"code":0}')


def server():
    s = http.server.HTTPServer(("127.0.0.1", 0), Service)
    s.visits, s.fail = [], False
    threading.Thread(target=s.serve_forever, daemon=True).start()
    return s


c, a, cloud = server(), server(), server()
addr = lambda s: "127.0.0.1:" + str(s.server_port)
env = dict(os.environ, FIXTURE_DIR=directory, AI_QUOTA_DATA_DIR=directory,
           CODEX_RADAR=addr(c), AGY_RADAR=addr(a), MOCK_CLOUD="http://" + addr(cloud), NO_PROXY="*")


def start(epoch):
    (root / "diagnostics.json").unlink(missing_ok=True)
    return subprocess.Popen([driver, "--ignored", "--exact", "fixture_companion"], env=dict(env, FIXTURE_EPOCH=str(epoch)))


def wait_for(check, timeout=6):
    until = time.monotonic() + timeout
    while time.monotonic() < until:
        if check():
            return
        time.sleep(0.015)
    raise AssertionError("timed out waiting for " + str(check))


def latest():
    return json.loads((root / "diagnostics.json").read_text())


def report():
    wait_for((root / "diagnostics.json").exists)
    output = subprocess.check_output([binary, "diagnostics"], env=env)
    for canary in [b"fixture-key", b"private-", b"csrf", b"rateLimits", bytes(directory, "utf8")]:
        assert canary not in output
    return json.loads(output)


def states(**changed):
    entries = report()
    assert "gemini" not in entries and "claude_gpt" not in entries
    for key in ("gemini_five_hour", "gemini_weekly"):
        assert entries[key]["status"] == changed.get(key, changed.get("antigravity_quota", "current")), entries
        assert entries[key]["last_success"] is not None
    for name in ("codex_quota", "antigravity_quota", "codex_radar", "antigravity_radar", "publisher"):
        assert entries[name]["status"] == changed.get(name, "current"), (name, entries)


def stop(p):
    p.send_signal(signal.SIGTERM)
    assert p.wait(timeout=6) == 0
    for pid in map(int, (root / "pids").read_text().split()):
        def gone():
            try:
                os.kill(pid, 0)
                return False
            except ProcessLookupError:
                return True
        wait_for(gone)


p = start(base)
try:
    wait_for(lambda: len(cloud.visits) == 2)
    wait_for(lambda: latest()["publisher"]["status"] == "current")
    states()
    assert cloud.visits[1] - cloud.visits[0] >= 60
    assert len(c.visits) == len(a.visits) == 1
    (root / "bad-codex").touch()
    wait_for(lambda: len((root / "reads").read_text().splitlines()) >= 2)
    wait_for(lambda: latest()["codex_quota"]["status"] == "stale")
    states(codex_quota="stale")
    (root / "bad-codex").unlink()
    (root / "status.json").write_text("invalid private-snapshot")
    wait_for(lambda: latest()["antigravity_quota"]["status"] == "stale")
    states(antigravity_quota="stale")
    (root / "status.json").write_text(snapshot)
    c.fail = True
    a.fail = False
    wait_for(lambda: len(c.visits) == 2, timeout=18)
    wait_for(lambda: latest()["codex_radar"]["status"] == "stale")
    states(codex_radar="stale")
    wait_for(lambda: len((root / "reads").read_text().splitlines()) == 7)
    assert len(a.visits) == 2
    # Initial frame, quota countdown, each quota failure, quota recovery, radar failure.
    assert len(cloud.visits) == 6, cloud.visits
finally:
    stop(p)

# Restore a valid Codex schedule through 304; the other radar now fails independently.
c.fail, a.fail = False, True
weekly_only = json.loads(snapshot)
del weekly_only["quota"]["gemini-5h"]
(root / "status.json").write_text(json.dumps(weekly_only))
p = start(base + 4200)
try:
    states(antigravity_radar="stale", antigravity_quota="stale", gemini_weekly="current")
    assert latest()["gemini_five_hour"]["reason"] == "missing_window"
finally:
    stop(p)

(root / "bad-codex").touch()
(root / "status.json").unlink()
cloud.fail = True
prior_success = json.loads((root / "publisher.json").read_text())["last_success"]
p = start(base + 4800)
try:
    states(codex_quota="stale", antigravity_quota="stale", antigravity_radar="stale", publisher="stale")
    assert json.loads((root / "publisher.json").read_text())["last_success"] == prior_success
finally:
    stop(p)
