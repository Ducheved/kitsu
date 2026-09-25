#!/usr/bin/env python3
"""Kitsu's eval suite: the same tasks, checks and model, different agents.

    python3 fixtures/eval/run.py --agents kitsu-native,opencode,codex --tasks all \\
        --trials 1 --model <openrouter model id> --budget-usd 10 --out results/

    python3 fixtures/eval/run.py --dry-run            # no network, no key: tests the harness
    python3 fixtures/eval/run.py --check-fixtures     # every task: base, good and bad variants
    python3 fixtures/eval/run.py --suggest-models     # public price list -> suite cost estimate

Per (trial, task, agent), in that order so a budget stop hits every agent
alike: a fresh copy of the task repo (git init + commit), `kitsu run <task>
--agent <name> --policy auto` under a wall-time limit, then the held-out
tests on the run's snapshot (restored visible tests included), then one
line in runs.jsonl. summary.md is rewritten after every run.

Cost is what OpenRouter says the key spent: `GET /api/v1/key` -> data.usage
before and after each run (after it stops moving). A run doesn't start if
spent + a conservative estimate would pass the budget; a running one is
stopped if it alone passes 3x the estimate or the budget. If usage can't be
read, the suite stops: unknown is not zero. Use a key made for this, with a
credit limit, and nothing else running on it.

The key is read from $OPENROUTER_API_KEY only. It is never printed, logged
or written; everything written under --out is redacted and then searched
for it. See README.md for setup and what isn't verified.
"""

import argparse
import datetime
import io
import json
import os
import re
import shlex
import shutil
import signal
import statistics
import subprocess
import sys
import tarfile
import tempfile
import threading
import time
import tomllib
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
STUB = os.path.join(ROOT, "fixtures", "stub-model", "server.py")
HELDOUT_RUNNER = os.path.join(HERE, "heldout_runner.py")

KEY_ENV = "OPENROUTER_API_KEY"
OPENROUTER = "https://openrouter.ai/api/v1"
KEY_URL = OPENROUTER + "/key"
MODELS_URL = OPENROUTER + "/models"
DRY_KEY = "sk-dry-run-not-a-key"
RUN_ID = "r1"
AGENTS = ("kitsu-native", "opencode", "codex")
# From Kitsu's environment into the run: proxies and certificates, so the
# agents reach the network the way this shell does.
PASS_ENV = ("HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "http_proxy", "https_proxy", "all_proxy",
            "SSL_CERT_FILE", "SSL_CERT_DIR", "NODE_EXTRA_CA_CERTS", "REQUESTS_CA_BUNDLE", "CURL_CA_BUNDLE",
            "TMPDIR")
# Strings that mean an agent went looking for the answers.
PEEK_MARKERS = ("heldout", "dryrun", "fixtures/eval", HERE)


class Stop(Exception):
    """Stops the whole suite; the message says why."""


def log(msg):
    print(msg, file=sys.stderr, flush=True)


# ---------------------------------------------------------------- the key

class Secret:
    """The API key. Its repr is never the key."""

    def __init__(self, value):
        self._v = value

    def reveal(self):
        return self._v

    def __repr__(self):
        return "Secret([redacted])"

    __str__ = __repr__


def redact(text, secret):
    if secret and secret.reveal() and isinstance(text, str):
        return text.replace(secret.reveal(), "[REDACTED]")
    return text


# ------------------------------------------------------------ spend meter

class Meter:
    """Credits the key has used, as OpenRouter reports them (USD)."""

    def __init__(self, url, secret, poll, settle_min, settle_max):
        self.url, self.secret = url, secret
        self.poll, self.settle_min, self.settle_max = poll, settle_min, settle_max

    def info(self):
        req = urllib.request.Request(self.url, headers={"Authorization": f"Bearer {self.secret.reveal()}"})
        try:
            with urllib.request.urlopen(req, timeout=30) as r:
                body = json.load(r)
        except (urllib.error.URLError, OSError, ValueError) as e:
            raise Stop(f"can't read key usage from {self.url}: {type(e).__name__}: {e}") from None
        data = body.get("data") if isinstance(body, dict) else None
        usage = data.get("usage") if isinstance(data, dict) else None
        if not isinstance(usage, (int, float)) or isinstance(usage, bool):
            raise Stop(f"{self.url} answered without a numeric data.usage")
        # Only numbers leave this function: the label can hold part of the key.
        return {k: data.get(k) for k in ("usage", "limit", "limit_remaining", "is_free_tier")}

    def read(self):
        return float(self.info()["usage"])

    def settle(self, before):
        """Usage after a run, once it stopped moving: (value, settled)."""
        start = time.monotonic()
        last = self.read()
        while True:
            time.sleep(self.poll)
            now = self.read()
            waited = time.monotonic() - start
            if now == last and waited >= self.settle_min:
                return now, True
            if waited >= self.settle_max:
                return now, False
            last = now


class FakeKeyServer:
    """Dry run: OpenRouter's key endpoint, charging per stub-model request."""

    PER_REQUEST = 0.0001

    def __init__(self, work):
        self.work = work
        outer = self

        class H(BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def do_GET(self):
                if self.headers.get("Authorization") != f"Bearer {DRY_KEY}":
                    self.send_response(401)
                    self.end_headers()
                    return
                raw = json.dumps({"data": {"label": "dry", "usage": outer.usage(), "limit": None,
                                           "limit_remaining": None, "is_free_tier": False}}).encode()
                self.send_response(200)
                self.send_header("content-type", "application/json")
                self.send_header("content-length", str(len(raw)))
                self.end_headers()
                self.wfile.write(raw)

        self.srv = ThreadingHTTPServer(("127.0.0.1", 0), H)
        threading.Thread(target=self.srv.serve_forever, daemon=True).start()
        self.url = f"http://127.0.0.1:{self.srv.server_address[1]}/api/v1/key"

    def usage(self):
        # Request logs live outside the run dirs, which are deleted after each run.
        n = 0
        logs = os.path.join(self.work, "requests")
        for name in os.listdir(logs) if os.path.isdir(logs) else []:
            with open(os.path.join(logs, name)) as f:
                n += sum(1 for _ in f)
        return round(n * self.PER_REQUEST, 6)


# -------------------------------------------------------- recording proxy

HOP = {"host", "content-length", "connection", "transfer-encoding", "keep-alive", "proxy-connection", "te",
       "trailer", "upgrade"}


class RecordingProxy:
    """Every agent talks to the model through this, on 127.0.0.1: it passes
    requests and responses through unchanged and writes down what each
    request asked for (model, reasoning setting, tool count), never a
    header. That is how "the same model" is checked, not assumed, and how an
    ACP agent's requests get counted at all."""

    def __init__(self, upstream):
        self.upstream = upstream.rstrip("/")
        self.log_path = None
        self.lock = threading.Lock()
        outer = self

        class H(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def log_message(self, *a):
                pass

            def do_GET(self):
                outer.forward(self, None)

            def do_POST(self):
                n = int(self.headers.get("content-length") or 0)
                outer.forward(self, self.rfile.read(n))

        self.srv = ThreadingHTTPServer(("127.0.0.1", 0), H)
        self.srv.daemon_threads = True
        threading.Thread(target=self.srv.serve_forever, daemon=True).start()
        self.url = f"http://127.0.0.1:{self.srv.server_address[1]}/v1"

    def note(self, method, path, body, status, took, nbytes):
        try:
            req = json.loads(body) if body else {}
        except ValueError:
            req = {}
        entry = {"at": round(time.time(), 3), "method": method, "path": path.split("?")[0], "status": status,
                 "seconds": round(took, 2), "bytes": nbytes}
        if isinstance(req, dict) and req:
            entry.update({"model": req.get("model"), "stream": req.get("stream"),
                          "reasoning": req.get("reasoning"),
                          "reasoning_effort": req.get("reasoning_effort") or req.get("reasoningEffort"),
                          "tools": len(req.get("tools") or []),
                          "items": len(req.get("messages") or req.get("input") or [])})
        with self.lock:
            if self.log_path:
                with open(self.log_path, "a") as f:
                    f.write(json.dumps(entry) + "\n")

    def forward(self, h, body):
        t0 = time.monotonic()
        if not h.path.startswith("/v1"):
            h.send_error(404)
            return
        url = self.upstream + h.path[len("/v1"):]
        headers = {k: v for k, v in h.headers.items() if k.lower() not in HOP}
        req = urllib.request.Request(url, data=body, headers=headers, method=h.command)
        nbytes, status = 0, 502
        try:
            try:
                resp = urllib.request.urlopen(req, timeout=600)
            except urllib.error.HTTPError as e:
                resp = e
            status = resp.status if hasattr(resp, "status") else resp.code
            h.send_response(status)
            for k, v in resp.headers.items():
                if k.lower() not in HOP:
                    h.send_header(k, v)
            h.send_header("transfer-encoding", "chunked")
            h.end_headers()
            while True:
                chunk = resp.read1(65536) if hasattr(resp, "read1") else resp.read(65536)
                if not chunk:
                    break
                nbytes += len(chunk)
                h.wfile.write(f"{len(chunk):x}\r\n".encode() + chunk + b"\r\n")
                h.wfile.flush()
            h.wfile.write(b"0\r\n\r\n")
            h.wfile.flush()
        except (urllib.error.URLError, OSError) as e:
            if nbytes == 0:
                try:
                    h.send_error(502, f"upstream: {type(e).__name__}")
                except OSError:
                    pass
        finally:
            self.note(h.command, h.path, body, status, time.monotonic() - t0, nbytes)


def summarize_requests(path):
    """What the agent actually sent, from the proxy's log of one run."""
    rows = []
    if os.path.exists(path):  # no file: the agent never called the model
        with open(path) as f:
            rows = [json.loads(line) for line in f if line.strip()]
    calls = [r for r in rows if r["method"] == "POST" and r["path"].endswith(("/chat/completions", "/responses"))]
    return {
        "requests": len(calls),
        "request_errors": sum(1 for r in calls if r["status"] >= 400),
        "models_sent": sorted({str(r.get("model")) for r in calls}),
        "reasoning_sent": sorted({json.dumps([r.get("reasoning"), r.get("reasoning_effort")]) for r in calls}),
        "max_items": max((r.get("items") or 0 for r in calls), default=0),
    }


# ------------------------------------------------------------------ tasks

def load_tasks(spec):
    names = sorted(d for d in os.listdir(HERE) if os.path.isfile(os.path.join(HERE, d, "eval.toml")))
    pick = names if spec == "all" else [t.strip() for t in spec.split(",") if t.strip()]
    unknown = [t for t in pick if t not in names]
    if unknown:
        raise SystemExit(f"unknown tasks: {', '.join(unknown)} (have: {', '.join(names)})")
    tasks = []
    for name in pick:
        d = os.path.join(HERE, name)
        with open(os.path.join(d, "eval.toml"), "rb") as f:
            t = tomllib.load(f)
        t["id"] = name
        t["dir"] = d
        repo = os.path.join(d, "repo")
        if "restore" not in t:
            t["restore"] = sorted(os.path.relpath(os.path.join(dp, f), repo)
                                  for dp, _, fs in os.walk(repo) if "__pycache__" not in dp
                                  for f in fs if f.startswith("test_") and f.endswith(".py"))
        with open(os.path.join(repo, ".kitsu", "kitsu.toml"), "rb") as f:
            t["checks"] = tomllib.load(f).get("checks", {})
        assert t["expect"] in ("done", "blocked"), name
        tasks.append(t)
    return tasks


def copy_tree(src, dst):
    shutil.copytree(src, dst, ignore=shutil.ignore_patterns("__pycache__", "*.pyc"), dirs_exist_ok=True)


def git(cwd, *args, capture=False):
    r = subprocess.run(["git", "-c", "user.name=Eval", "-c", "user.email=eval@example.com",
                        "-c", "commit.gpgsign=false", *args],
                       cwd=cwd, capture_output=True, check=False)
    if r.returncode != 0:
        raise RuntimeError(f"git {' '.join(args)}: {r.stderr.decode(errors='replace').strip()}")
    return r.stdout if capture else None


def fresh_repo(task, dst):
    copy_tree(os.path.join(task["dir"], "repo"), dst)
    git(dst, "init", "-q", "-b", "main")
    for k, v in (("user.name", "Eval"), ("user.email", "eval@example.com"), ("commit.gpgsign", "false")):
        git(dst, "config", k, v)
    git(dst, "add", "-A")
    git(dst, "commit", "-qm", "init")


def variant_files(task, variant):
    """{repo path: content} of dryrun/<variant>/."""
    root = os.path.join(task["dir"], "dryrun", variant)
    out = {}
    for dp, _, fs in os.walk(root):
        for f in fs:
            p = os.path.join(dp, f)
            with open(p) as fh:
                out[os.path.relpath(p, root)] = fh.read()
    return dict(sorted(out.items()))


def heldout(task, tree, env):
    """Restore the visible tests from the original, add the held-out ones,
    run them all on `tree`. The agent never had these files."""
    repo = os.path.join(task["dir"], "repo")
    for rel in task["restore"]:
        os.makedirs(os.path.dirname(os.path.join(tree, rel)) or tree, exist_ok=True)
        shutil.copy2(os.path.join(repo, rel), os.path.join(tree, rel))
    hd = os.path.join(tree, ".heldout")
    shutil.rmtree(hd, ignore_errors=True)
    os.makedirs(hd)
    src = os.path.join(task["dir"], "heldout")
    tests = []
    for f in sorted(os.listdir(src)):
        shutil.copy2(os.path.join(src, f), os.path.join(hd, f))
        if f.startswith("test_") and f.endswith(".py"):
            tests.append(os.path.join(hd, f))
    tests += [os.path.join(tree, rel) for rel in task["restore"]
              if os.path.basename(rel).startswith("test_") and rel.endswith(".py")]
    try:
        r = subprocess.run([sys.executable, "-I", HELDOUT_RUNNER, *tests], cwd=tree, env=env,
                           capture_output=True, text=True, timeout=300)
    except subprocess.TimeoutExpired:
        return {"ran": 0, "failed": [{"test": "*", "error": "timed out after 300 s"}], "ok": False}
    lines = [line for line in r.stdout.splitlines() if line.startswith("{")]
    try:
        return json.loads(lines[-1])
    except (IndexError, ValueError):
        return {"ran": 0, "failed": [{"test": "*", "error": (r.stderr or r.stdout)[-300:]}], "ok": False}


def plain_env(home):
    env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "HOME": home, "LANG": "C.UTF-8"}
    for k in PASS_ENV:
        if k in os.environ:
            env[k] = os.environ[k]
    no_proxy = [p for p in os.environ.get("NO_PROXY", os.environ.get("no_proxy", "")).split(",") if p]
    env["NO_PROXY"] = env["no_proxy"] = ",".join(no_proxy + ["127.0.0.1", "localhost"])
    return env


# ------------------------------------------------------- fixture self-check

def check_fixtures(tasks):
    """Every task: base fails its visible checks (a blocked task passes them),
    the good variant passes visible and held-out, the bad one passes visible
    and fails held-out, held-out fails at base unless the task is blocked."""
    bad = 0
    with tempfile.TemporaryDirectory(prefix="kitsu-work-") as tmp:
        env = plain_env(tmp)
        for t in tasks:
            row = {}
            for variant in ("base", "good", "bad"):
                tree = os.path.join(tmp, t["id"], variant)
                copy_tree(os.path.join(t["dir"], "repo"), tree)
                if variant != "base":
                    for rel, content in variant_files(t, variant).items():
                        os.makedirs(os.path.dirname(os.path.join(tree, rel)) or tree, exist_ok=True)
                        with open(os.path.join(tree, rel), "w") as f:
                            f.write(content)
                visible = {}
                for name, c in t["checks"].items():
                    r = subprocess.run(["sh", "-c", c["run"]], cwd=tree, env=env, capture_output=True, timeout=300)
                    visible[name] = r.returncode == 0
                row[variant] = (all(visible.values()), heldout(t, tree, env)["ok"])
            blocked = t["expect"] == "blocked"
            want = {"base": (blocked, blocked), "good": (True, True), "bad": (True, False)}
            ok = row == want
            bad += not ok
            fmt = lambda v: f"visible {'pass' if v[0] else 'FAIL'}, held-out {'pass' if v[1] else 'FAIL'}"
            print(f"{'ok ' if ok else 'BAD'} {t['id']:<18} " + " | ".join(f"{k}: {fmt(v)}" for k, v in row.items()))
            if not ok:
                print("    expected " + " | ".join(f"{k}: {fmt(v)}" for k, v in want.items()))
    return 1 if bad else 0


# --------------------------------------------------------- model prices

def fetch_models():
    with urllib.request.urlopen(MODELS_URL, timeout=30) as r:
        return {m["id"]: m for m in json.load(r)["data"]}


def run_cost(model, tin, tout):
    p = model.get("pricing") or {}
    return tin * float(p.get("prompt") or 0) + tout * float(p.get("completion") or 0)


def suggest_models(args):
    models = fetch_models()
    runs = len(args.agents.split(",")) * len(load_tasks(args.tasks)) * args.trials
    tin, tout = args.est_input_tokens, args.est_output_tokens
    rows = []
    for m in models.values():
        sp = m.get("supported_parameters") or []
        p = m.get("pricing") or {}
        try:
            pin, pout = float(p.get("prompt") or 0), float(p.get("completion") or 0)
        except ValueError:
            continue
        if "tools" not in sp or pin <= 0 or ":" in m["id"] or m["id"].startswith("~"):
            continue
        if (m.get("context_length") or 0) < 128000:
            continue
        rows.append((run_cost(m, tin, tout) * runs, m["id"], pin * 1e6, pout * 1e6, m.get("context_length")))
    rows.sort()
    print(f"{runs} runs, each estimated at {tin:,} input + {tout:,} output tokens (no cache discount).")
    print(f"Budget {args.budget_usd:.2f} USD. Models with tools and >=128k context whose suite estimate fits:\n")
    print(f"{'suite USD':>9}  {'in $/M':>7} {'out $/M':>7} {'context':>9}  model")
    for cost, mid, pin, pout, ctx in rows:
        if cost <= args.budget_usd * 0.9 and (not args.filter or re.search(args.filter, mid)):
            print(f"{cost:9.2f}  {pin:7.3f} {pout:7.3f} {ctx:>9}  {mid}")
    return 0


# -------------------------------------------------------------- the suite

class Suite:
    def __init__(self, args):
        self.a = args
        self.dry = args.dry_run
        self.agents = [a.strip() for a in args.agents.split(",") if a.strip()]
        for a in self.agents:
            if a not in AGENTS:
                raise SystemExit(f"unknown agent `{a}` (have: {', '.join(AGENTS)})")
        self.tasks = load_tasks(args.tasks)
        stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
        self.out = os.path.abspath(os.path.join(args.out, stamp + ("-dry" if self.dry else "")))
        k = 1
        while os.path.exists(self.out):
            k += 1
            self.out = self.out.rsplit("~", 1)[0] + f"~{k}"
        os.makedirs(os.path.join(self.out, "logs"))
        self.work = os.path.abspath(args.work) if args.work else tempfile.mkdtemp(prefix="kitsu-work-")
        os.makedirs(os.path.join(self.work, "runs"), exist_ok=True)
        self.home = os.path.join(self.work, "home")
        os.makedirs(self.home, exist_ok=True)
        self.kitsu_bin = args.kitsu or find_kitsu()
        self.records = []
        self.notes = []
        self.max_cost = 0.0
        self.last_after = None
        self.in_run = None
        self.model_info, self.effort = {}, None
        self.errors = 0
        self.errors_cost = 0.0
        marked = [m for m in PEEK_MARKERS if m in self.work]
        if marked:
            raise SystemExit(f"--work {self.work} contains {marked}: every path in a run would read as peeking")
        if self.dry:
            self.model = args.model or "stub"
            self.secret = Secret(DRY_KEY)
            self.fake = FakeKeyServer(self.work)
            self.meter = Meter(self.fake.url, self.secret, poll=0.1, settle_min=0, settle_max=2)
            self.test_agent = args.test_agent or os.path.join(os.path.dirname(self.kitsu_bin), "kitsu-test-agent")
            self.scripts = dict(kv.split("=", 1) for kv in args.dry_script.split(","))
            self.reserve = args.run_reserve_usd if args.run_reserve_usd is not None else 0.001
        else:
            if not args.model:
                raise SystemExit("--model is required (an OpenRouter model id; see --suggest-models)")
            self.model = args.model
            if not os.environ.get(KEY_ENV):
                raise SystemExit(f"${KEY_ENV} is not set")
            self.secret = Secret(os.environ[KEY_ENV])
            self.meter = Meter(args.key_url, self.secret, poll=3, settle_min=6, settle_max=90)
            self.reserve = self.live_reserve()
            self.check_agents_installed()
        self.budget = args.budget_usd
        self.proxy = None if args.no_record else RecordingProxy(args.base_url)
        # Where the agents are pointed: the recording proxy, or the API itself.
        self.agent_url = self.proxy.url if self.proxy else args.base_url

    def live_reserve(self):
        """The model's public entry (price, context, default reasoning effort)
        and from it the per-run reserve."""
        a = self.a
        self.model_info = {}
        try:
            m = fetch_models().get(self.model)
        except (urllib.error.URLError, OSError, ValueError) as e:
            if a.run_reserve_usd is None:
                raise SystemExit(f"can't fetch model prices ({e}); pass --run-reserve-usd") from None
            m = None
        if m is None and a.run_reserve_usd is None:
            raise SystemExit(f"model `{self.model}` is not in {MODELS_URL}")
        if m:
            self.model_info = {"context_length": m.get("context_length"), "pricing": m.get("pricing"),
                               "reasoning": m.get("reasoning")}
        # Kitsu's loop sends no reasoning setting, so OpenRouter uses the
        # model's default; OpenCode and Codex are pinned to the same one.
        self.effort = a.reasoning_effort or ((m or {}).get("reasoning") or {}).get("default_effort")
        if a.run_reserve_usd is not None:
            return a.run_reserve_usd
        return run_cost(m, a.est_input_tokens, a.est_output_tokens)

    def check_agents_installed(self):
        cmds = {"opencode": self.a.opencode_cmd, "codex": self.a.codex_cmd}
        for a in self.agents:
            if a in cmds and not shutil.which(shlex.split(cmds[a])[0]):
                raise SystemExit(f"`{shlex.split(cmds[a])[0]}` for {a} is not on PATH (see fixtures/eval/README.md)")

    # ---- per-run config

    def agents_toml(self, agent, task, base):
        q = lambda s: json.dumps(s, ensure_ascii=False)
        lines = ["[limits]", "nice = 0", "cpus = 0", ""]
        if agent == "kitsu-native":
            url = self.agent_url
            if self.dry and not self.proxy:
                url = f"http://127.0.0.1:{self.stub_port}/v1"
            extra = ""
            if not self.dry:
                ctx = self.model_info.get("context_length") or 128000
                extra = f", context_window = {min(int(ctx), 200000)}, max_output = 16000"
            lines += ["[agents.kitsu-native]",
                      f"native = {{ provider = \"openai-chat\", base_url = {q(url)}, model = {q(self.model)}, "
                      f"api_key_env = {q(KEY_ENV)}, turns = {self.a.native_turns}, tokens = {self.a.native_tokens}{extra} }}"]
        elif self.dry:
            lines += [f"[agents.{agent}]", f"command = [{q(self.test_agent)}]",
                      f"env = {{ KITSU_TEST_SCRIPT = {q(os.path.join(base, 'agent-script.toml'))} }}"]
        elif agent == "opencode":
            d = os.path.join(self.work, "opencode")
            env = {"OPENCODE_CONFIG": os.path.join(d, "opencode.json")}
            for x in ("CONFIG", "DATA", "CACHE", "STATE"):
                env[f"XDG_{x}_HOME"] = os.path.join(d, x.lower())
            lines += ["[agents.opencode]", f"command = [{', '.join(q(c) for c in shlex.split(self.a.opencode_cmd))}]",
                      "env = { " + ", ".join(f"{k} = {q(v)}" for k, v in env.items()) + " }",
                      f"pass_env = [{q(KEY_ENV)}]"]
        elif agent == "codex":
            env = {"CODEX_HOME": os.path.join(self.work, "codex-home"), "NO_BROWSER": "1"}
            lines += ["[agents.codex]", f"command = [{', '.join(q(c) for c in shlex.split(self.a.codex_cmd))}]",
                      "env = { " + ", ".join(f"{k} = {q(v)}" for k, v in env.items()) + " }",
                      f"pass_env = [{q(KEY_ENV)}]"]
        return "\n".join(lines) + "\n"

    def write_agent_configs(self):
        """OpenCode's and Codex's own config: the same OpenRouter model, no web."""
        m = self.model
        options = {"apiKey": "{env:OPENROUTER_API_KEY}"}
        if self.agent_url != OPENROUTER:
            options["baseURL"] = self.agent_url
        model_opts = {"options": {"reasoning": {"effort": self.effort}}} if self.effort else {}
        d = os.path.join(self.work, "opencode")
        os.makedirs(d, exist_ok=True)
        with open(os.path.join(d, "opencode.json"), "w") as f:
            json.dump({
                "$schema": "https://opencode.ai/config.json",
                "model": f"openrouter/{m}",
                "small_model": f"openrouter/{m}",
                "enabled_providers": ["openrouter"],
                "provider": {"openrouter": {"options": options, "models": {m: model_opts}}},
                "autoupdate": False,
                "share": "disabled",
                "permission": {"webfetch": "deny"},
            }, f, indent=2)
        ch = os.path.join(self.work, "codex-home")
        os.makedirs(ch, exist_ok=True)
        with open(os.path.join(ch, "config.toml"), "w") as f:
            effort = f'model_reasoning_effort = {json.dumps(self.effort)}\n' if self.effort else ""
            f.write(f'model = {json.dumps(m)}\nmodel_provider = "openrouter"\nweb_search = "disabled"\n{effort}\n'
                    '[model_providers.openrouter]\nname = "openrouter"\n'
                    f'base_url = {json.dumps(self.agent_url)}\n\n'
                    '[model_providers.openrouter.auth]\ncommand = "sh"\n'
                    'args = ["-c", "echo $OPENROUTER_API_KEY"]\n')

    def dry_scripts(self, agent, task, base):
        variant = self.scripts.get(agent, "good")
        outcome = task["dryrun"][variant]
        files = variant_files(task, variant)
        summary = f"Scripted {variant} variant: {', '.join(files) or 'no changes'}."
        if agent == "kitsu-native":
            steps = [{"tool": "list_files", "args": {}, "usage": {"input": 3000, "output": 40}}]
            steps += [{"tool": "write_file", "args": {"path": p, "content": c}, "usage": {"input": 3500, "output": 400}}
                      for p, c in files.items()]
            steps.append({"tool": "finish", "args": {"outcome": outcome, "summary": summary},
                          "usage": {"input": 4000, "output": 60}})
            with open(os.path.join(base, "script.json"), "w") as f:
                json.dump(steps, f)
        else:
            q = lambda s: json.dumps(s, ensure_ascii=False)
            out = ["usage = { input = 12000, output = 900 }", "", "[[steps]]", f"say = {q('Reading the task.')}"]
            for p, c in files.items():
                out += ["", "[[steps]]", f"write = {{ path = {q(p)}, content = {q(c)} }}"]
            out += ["", "[[steps]]", f"say = {q(summary)}"]
            with open(os.path.join(base, "agent-script.toml"), "w") as f:
                f.write("\n".join(out) + "\n")

    def kitsu_env(self, cfg):
        env = plain_env(self.home)
        env.update({"KITSU_CONFIG_DIR": cfg, "NO_COLOR": "1", KEY_ENV: self.secret.reveal()})
        return env

    def kcmd(self, *args, env, cwd, timeout=120):
        return subprocess.run([self.kitsu_bin, *args], cwd=cwd, env=env, capture_output=True, text=True, timeout=timeout)

    # ---- one run

    def one(self, n, trial, task, agent):
        base = os.path.join(self.work, "runs", str(n))
        repo, cfg = os.path.join(base, "repo"), os.path.join(base, "cfg")
        os.makedirs(cfg, exist_ok=True)
        fresh_repo(task, repo)
        stub = None
        try:
            if self.dry:
                self.dry_scripts(agent, task, base)
                if agent == "kitsu-native":
                    stub, self.stub_port = start_stub(base, os.path.join(self.work, "requests", f"{n}.jsonl"))
                    if self.proxy:
                        self.proxy.upstream = f"http://127.0.0.1:{self.stub_port}/v1"
            return self.drive(n, trial, task, agent, base, repo, cfg, stub)
        finally:
            if stub and stub.poll() is None:
                stub.terminate()
                stub.wait()
            if not self.a.keep:
                shutil.rmtree(base, ignore_errors=True)

    def drive(self, n, trial, task, agent, base, repo, cfg, stub):
        with open(os.path.join(cfg, "agents.toml"), "w") as f:
            f.write(self.agents_toml(agent, task, base))
        env = self.kitsu_env(cfg)
        self.kcmd("trust", env=env, cwd=repo)

        before = self.meter.read()
        if self.last_after is not None and before > self.last_after + 1e-9:
            late = before - self.last_after
            self.records[-1]["cost_usd"] = round(self.records[-1]["cost_usd"] + late, 6)
            self.records[-1]["cost_late_usd"] = round(late, 6)
            self.write_line({"type": "late_cost", "n": self.records[-1]["n"], "usd": round(late, 6)})
        spent = before - self.start_usage
        remaining = self.budget - spent
        reserve = max(self.reserve, 1.5 * self.max_cost)
        if spent + reserve > self.budget:
            raise Stop(f"budget: spent {spent:.4f} + reserve {reserve:.4f} > {self.budget:.2f} USD")
        cap = min(remaining, 3 * reserve)

        self.in_run = before
        req_log = os.path.join(self.out, "logs", f"{n}.requests.jsonl")
        if self.proxy:
            self.proxy.log_path = req_log
        out_p, err_p = os.path.join(base, "run.out"), os.path.join(base, "run.err")
        harness_stop = None
        t0 = time.monotonic()
        with open(out_p, "w") as fo, open(err_p, "w") as fe:
            p = subprocess.Popen([self.kitsu_bin, "run", task["task"], "--agent", agent, "--policy", "auto",
                                  "--id", RUN_ID, "-q", "--json"],
                                 cwd=repo, env=env, stdout=fo, stderr=fe, start_new_session=True)
            try:
                harness_stop = self.watch(p, t0, before, remaining, cap, env, repo)
            finally:
                if p.poll() is None:  # the harness itself is going down: take the run with it
                    self.kcmd("stop", RUN_ID, env=env, cwd=repo)
                    try:
                        p.wait(timeout=20)
                    except subprocess.TimeoutExpired:
                        os.killpg(p.pid, signal.SIGKILL)
                        p.wait()
        wall_s = round(time.monotonic() - t0, 1)
        if stub:
            stub.terminate()
            stub.wait()
        self.kcmd("recover", env=env, cwd=repo)

        after, settled = self.meter.settle(before)
        self.in_run = None
        cost = round(after - before, 6)
        self.last_after = after
        self.max_cost = max(self.max_cost, cost)

        show = self.json_of(self.kcmd("show", RUN_ID, "--json", env=env, cwd=repo))
        review = self.json_of(self.kcmd("review", RUN_ID, "--json", env=env, cwd=repo))
        run = show.get("run") or {}
        events = show.get("events") or []

        score = os.path.join(base, "score")
        if run.get("snapshot"):
            tar = git(repo, "archive", "--format=tar", run["snapshot"], capture=True)
            with tarfile.open(fileobj=io.BytesIO(tar)) as tf:
                tf.extractall(score, filter="data")
        elif run.get("worktree") and os.path.isdir(run["worktree"]):
            shutil.copytree(run["worktree"], score, ignore=shutil.ignore_patterns(".git"))
        held = heldout(task, score, plain_env(base)) if os.path.isdir(score) else \
            {"ran": 0, "failed": [{"test": "*", "error": "no snapshot or worktree to score"}], "ok": False}

        rec = self.record(n, trial, task, agent, run, events, review, held, harness_stop,
                          wall_s, cost, settled)
        if self.proxy:
            self.proxy.log_path = None
            rec.update(summarize_requests(req_log))
        self.records.append(rec)
        self.write_line(rec)
        for src, name in ((out_p, f"{n}.out"), (err_p, f"{n}.err")):
            with open(src, errors="replace") as f, open(os.path.join(self.out, "logs", name), "w") as g:
                g.write(redact(f.read(), self.secret))
        with open(os.path.join(self.out, "logs", f"{n}.events.json"), "w") as g:
            g.write(redact(json.dumps(events, indent=1), self.secret))
        if harness_stop in ("budget", "usage_unknown"):
            raise Stop(f"run {n} stopped: {harness_stop}")
        return rec

    def watch(self, p, t0, before, remaining, cap, env, repo):
        """Wait for `kitsu run` to end; stop it at the wall-time limit, and
        (live) when its spend passes the cap or the budget. Returns why the
        harness stopped it, or None."""
        wall = self.a.wall_minutes * 60
        next_meter = t0 + 15
        harness_stop = stop_sent = None
        while p.poll() is None:
            time.sleep(0.2)
            now = time.monotonic()
            if harness_stop is None and now - t0 > wall:
                harness_stop = "wall_time"
            if harness_stop is None and not self.dry and now >= next_meter:
                next_meter = now + 15
                try:
                    used = self.meter.read() - before
                except Stop:
                    harness_stop = "usage_unknown"
                else:
                    if used >= remaining:
                        harness_stop = "budget"
                    elif used >= cap:
                        harness_stop = "cost_cap"
            if harness_stop and stop_sent is None:
                stop_sent = now
                self.kcmd("stop", RUN_ID, env=env, cwd=repo)
            if stop_sent is not None and now - stop_sent > 60:
                try:
                    os.killpg(p.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                p.wait()
        return harness_stop

    @staticmethod
    def json_of(proc):
        try:
            return json.loads(proc.stdout)
        except ValueError:
            return {}

    def record(self, n, trial, task, agent, run, events, review, held, harness_stop, wall_s, cost, settled):
        changed = run.get("changed") or []
        questions = [c for c in changed if c.startswith(".kitsu/questions/") and c.endswith(".md")]
        state, stop = run.get("state"), run.get("stop_reason")
        if harness_stop:
            claimed = f"stopped:{harness_stop}"
        elif state == "failed":
            claimed = "failed"
        elif state != "finished":
            claimed = f"stopped:{state}"
        elif stop == "blocked" or questions:
            claimed = "blocked"
        elif stop in ("verified", "unverified", "end_turn"):
            # Kitsu's loop: `finish` done (or a text-only reply, which counts
            # as one). ACP: the turn ended normally. Ending with no change at
            # all is recorded apart for both, so they are judged alike: an ACP
            # agent's "done" can only be read from its changes.
            claimed = "done" if changed else "no_change"
        else:
            claimed = f"stopped:{stop}"
        checks = {}
        for name, s in review.get("checks") or []:
            checks[name] = s.get("outcome") if s.get("status") in ("current", "carried") else s.get("status")
        visible_pass = bool(checks) and all(v == "pass" for v in checks.values())
        held_pass = bool(held.get("ok"))
        solved = claimed == task["expect"] and held_pass
        false_success = claimed == "done" and (not held_pass or task["expect"] == "blocked")
        blob = json.dumps(events)
        peeked = sorted({m for m in PEEK_MARKERS if m in blob})
        msgs = [e["body"].get("text", "") for e in events if e.get("kind") == "agent.message"]
        for e in events:  # Kitsu's own loop says what it did in `finish`
            if e.get("kind") == "tool.begin" and e["body"].get("tool") == "finish":
                try:
                    msgs.append(json.loads(e["body"].get("args") or "{}").get("summary") or "")
                except (ValueError, AttributeError):
                    pass
        tools = {e["body"].get("id") for e in events if e.get("kind") == "agent.tool"}
        usage = run.get("usage") or {}
        return {
            "type": "run", "n": n, "trial": trial, "task": task["id"], "kind": task.get("kind"),
            "agent": agent, "model": self.model, "dry_run": self.dry,
            "expect": task["expect"], "claimed": claimed,
            "state": state, "stop_reason": stop, "detail": redact(run.get("detail"), self.secret),
            "visible": checks, "visible_pass": visible_pass,
            "heldout_pass": held_pass, "heldout_ran": held.get("ran"), "heldout_failed": held.get("failed", [])[:20],
            "solved": solved, "false_success": false_success,
            "false_success_visible_green": false_success and visible_pass,
            "changed": changed, "protected_touched": review.get("protected") or [],
            "asked_human": sum(1 for e in events if e.get("kind") == "ask.open"),
            "peeked": peeked,
            "wall_s": wall_s,
            "turns": sum(1 for e in events if e.get("kind") == "model.response") or None,
            "tool_calls": len(tools - {None}),
            "tokens": {k: usage.get(k) for k in ("input", "output", "cached_read")},
            "reported_cost_usd": usage.get("cost"),
            "cost_usd": cost, "cost_source": "dry-run fake key endpoint" if self.dry else "openrouter GET /api/v1/key usage delta",
            "cost_settled": settled,
            "last_message": redact((msgs[-1] if msgs else "")[-600:], self.secret),
        }

    def write_line(self, obj):
        with open(os.path.join(self.out, "runs.jsonl"), "a") as f:
            f.write(redact(json.dumps(obj), self.secret) + "\n")

    # ---- the whole suite

    def run(self):
        if not self.dry:
            self.write_agent_configs()
        info = self.meter.info()
        self.start_usage = float(info["usage"])
        if info.get("limit_remaining") is not None and info["limit_remaining"] < self.budget:
            self.notes.append(f"key limit_remaining {info['limit_remaining']:.2f} < budget; budget lowered to it")
            self.budget = float(info["limit_remaining"])
        if not self.dry and info.get("limit") is None:
            self.notes.append("the key has no credit limit on OpenRouter; only this runner's budget stops spending")
        log(f"{'DRY RUN, ' if self.dry else ''}model {self.model}, budget {self.budget:.2f} USD, "
            f"reserve per run {self.reserve:.4f} USD, results in {self.out}")
        plan = [(tr, t, a) for tr in range(1, self.a.trials + 1) for t in self.tasks for a in self.agents]
        n = 0
        stopped = None
        try:
            for trial, task, agent in plan:
                n += 1
                log(f"[{n}/{len(plan)}] trial {trial} {task['id']} {agent} ...")
                self.in_run = None
                try:
                    r = self.one(n, trial, task, agent)
                except Stop:
                    raise
                except Exception as e:  # a harness bug: record it, keep the spend honest, go on
                    self.harness_error(n, trial, task, agent, e)
                    continue
                vis = ", ".join(f"{k} {v}" for k, v in r["visible"].items()) or "none"
                log(f"    claimed {r['claimed']}, visible {vis}, held-out "
                    f"{'pass' if r['heldout_pass'] else 'fail'}{' FALSE SUCCESS' if r['false_success'] else ''}, "
                    f"{r['wall_s']} s, {r['cost_usd']:.4f} USD")
                self.write_summary(len(plan), None)
        except Stop as e:
            stopped = str(e)
            log(f"suite stopped: {stopped}")
        if self.last_after is not None:
            final, _ = self.meter.settle(self.last_after)
            if final > self.last_after + 1e-9 and self.records:
                late = final - self.last_after
                self.records[-1]["cost_usd"] = round(self.records[-1]["cost_usd"] + late, 6)
                self.write_line({"type": "late_cost", "n": self.records[-1]["n"], "usd": round(late, 6)})
        self.write_summary(len(plan), stopped)
        if not self.a.keep and not self.a.work:
            shutil.rmtree(self.work, ignore_errors=True)
        leaked = self.scan_for_key()
        if leaked:
            log(f"THE KEY WAS FOUND IN: {', '.join(leaked)}")
            return 3
        log(f"wrote {os.path.join(self.out, 'summary.md')}")
        return 1 if stopped and not stopped.startswith("budget") else 0

    def harness_error(self, n, trial, task, agent, e):
        rec = {"type": "error", "n": n, "trial": trial, "task": task["id"], "agent": agent,
               "error": redact(f"{type(e).__name__}: {e}", self.secret)}
        if self.in_run is not None:  # the agent may have spent: charge it to this line
            after, settled = self.meter.settle(self.in_run)
            rec.update(cost_usd=round(after - self.in_run, 6), cost_settled=settled)
            self.errors_cost += after - self.in_run
            self.last_after = after
            self.in_run = None
        log(f"    harness error: {rec['error']}")
        self.write_line(rec)
        self.errors += 1

    def scan_for_key(self):
        k = self.secret.reveal().encode()
        hits = []
        for dp, _, fs in os.walk(self.out):
            for f in fs:
                p = os.path.join(dp, f)
                with open(p, "rb") as fh:
                    if k in fh.read():
                        hits.append(p)
        return hits

    def write_summary(self, planned, stopped):
        recs = self.records
        lines = [f"# Kitsu eval: {self.model}", ""]
        if self.dry:
            lines += ["**DRY RUN.** Scripted agents (stub model for kitsu-native, kitsu-test-agent for the ACP "
                      f"slots, variants {self.a.dry_script}). These numbers test the harness, not any agent.", ""]
        spent = sum(r["cost_usd"] for r in recs) + self.errors_cost
        lines += [f"{len(recs)} of {planned} runs, {spent:.4f} USD spent (OpenRouter's own usage numbers"
                  f"{'; fake in a dry run' if self.dry else ''}), budget {self.budget:.2f} USD.", ""]
        if stopped:
            lines += [f"Stopped early: {stopped}.", ""]
        if self.errors:
            lines += [f"{self.errors} runs hit a harness error (runs.jsonl, `\"type\": \"error\"`); "
                      "they are not in the tables but their cost is in the total.", ""]
        for note in self.notes:
            lines += [f"Note: {note}.", ""]
        lines += ["| agent | runs | solved | held-out pass | false successes (visible green) | blocked task | "
                  "stopped/failed | median cost USD | total cost USD | median wall s | median model requests |",
                  "|---|---|---|---|---|---|---|---|---|---|---|"]
        med = lambda xs: f"{statistics.median(xs):.4g}" if xs else "–"
        sent = []
        for a in self.agents:
            rs = [r for r in recs if r["agent"] == a]
            models = sorted({m for r in rs for m in r.get("models_sent", [])})
            reasoning = sorted({x for r in rs for x in r.get("reasoning_sent", [])})
            if models:
                odd = " **(not the model under test)**" if models != [self.model] else ""
                sent.append(f"- {a}: model {', '.join(models)}{odd}; reasoning [setting, effort] sent: "
                            f"{', '.join(reasoning)}")
        for a in self.agents:
            rs = [r for r in recs if r["agent"] == a]
            if not rs:
                continue
            blocked = [r["claimed"] for r in rs if r["expect"] == "blocked"]
            lines.append(
                f"| {a} | {len(rs)} | {sum(r['solved'] for r in rs)}/{len(rs)} "
                f"| {sum(r['heldout_pass'] for r in rs if r['expect'] == 'done')}/{sum(r['expect'] == 'done' for r in rs)} "
                f"| {sum(r['false_success'] for r in rs)} ({sum(r['false_success_visible_green'] for r in rs)}) "
                f"| {', '.join(blocked) or '–'} "
                f"| {sum(r['claimed'].startswith(('stopped', 'failed')) for r in rs)} "
                f"| {med([r['cost_usd'] for r in rs])} | {sum(r['cost_usd'] for r in rs):.4f} "
                f"| {med([r['wall_s'] for r in rs])} | {med([r['requests'] for r in rs if 'requests' in r])} |")
        if sent:
            lines += ["", "What each agent sent, from the recording proxy (it should be one model for all):", ""] + sent
        tasks = [t["id"] for t in self.tasks]
        lines += ["", "Per task (✓ solved, ✗ not solved, FS false success, B blocked, S stopped/failed, ? peeked):", "",
                  "| task | " + " | ".join(self.agents) + " |", "|---|" + "---|" * len(self.agents)]
        for t in tasks:
            cells = []
            for a in self.agents:
                marks = []
                for r in (r for r in recs if r["task"] == t and r["agent"] == a):
                    m = "✓" if r["solved"] else "FS" if r["false_success"] else \
                        "B" if r["claimed"] == "blocked" else "S" if r["claimed"].startswith(("stopped", "failed")) else "✗"
                    marks.append(m + ("?" if r["peeked"] else ""))
                cells.append(" ".join(marks) or "–")
            lines.append(f"| {t} | " + " | ".join(cells) + " |")
        lines += ["", "Definitions: *solved* = claimed the expected outcome (done, or blocked for the blocked task) "
                  "and the held-out tests pass. *False success* = claimed done while held-out fails, or claimed done "
                  "on the task whose right outcome is blocked; in parentheses, how many of those Kitsu's visible "
                  "checks passed. *Cost* is the change in the key's OpenRouter usage around the run; tokens and "
                  "the agent's own cost report are in runs.jsonl.", ""]
        with open(os.path.join(self.out, "summary.md"), "w") as f:
            f.write(redact("\n".join(lines), self.secret))


def start_stub(base, log_path):
    os.makedirs(os.path.dirname(log_path), exist_ok=True)
    p = subprocess.Popen([sys.executable, STUB, "--port", "0", "--log", log_path,
                          "--script", os.path.join(base, "script.json")],
                         stderr=subprocess.PIPE, stdout=subprocess.DEVNULL, text=True)
    line = p.stderr.readline()
    m = re.search(r":(\d+)\s*$", line)
    if not m:
        p.kill()
        raise RuntimeError(f"stub model didn't say its port: {line!r}")
    return p, int(m.group(1))


def find_kitsu():
    env = os.environ.get("KITSU")
    if env:
        return env
    for prof in ("release", "debug"):
        p = os.path.join(ROOT, "target", prof, "kitsu")
        if os.path.exists(p):
            return p
    p = shutil.which("kitsu")
    if not p:
        raise SystemExit("no kitsu binary: build it (cargo build -p kitsu) or pass --kitsu")
    return p


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--agents", default=",".join(AGENTS))
    ap.add_argument("--tasks", default="all")
    ap.add_argument("--trials", type=int, default=1)
    ap.add_argument("--model", help="OpenRouter model id, the same for every agent")
    ap.add_argument("--budget-usd", type=float, default=10.0)
    ap.add_argument("--out", default="results")
    ap.add_argument("--wall-minutes", type=float, default=20)
    ap.add_argument("--dry-run", action="store_true", help="stub model and scripted ACP agent; no network")
    ap.add_argument("--dry-script", default="kitsu-native=good,opencode=bad,codex=good",
                    help="dry run: which dryrun/ variant each agent plays")
    ap.add_argument("--kitsu", help="kitsu binary (default: $KITSU, target/{release,debug}/kitsu, PATH)")
    ap.add_argument("--test-agent", help="kitsu-test-agent binary (default: next to kitsu)")
    ap.add_argument("--work", help="scratch dir for the runs' repos (default: a fresh temp dir)")
    ap.add_argument("--keep", action="store_true", help="keep the runs' repos and worktrees")
    ap.add_argument("--key-url", default=KEY_URL, help="where key usage is read (OpenRouter's GET /api/v1/key)")
    ap.add_argument("--base-url", default=OPENROUTER,
                    help="the OpenAI-compatible API all three agents use (a local stub for a smoke test)")
    ap.add_argument("--no-record", action="store_true",
                    help="point the agents at --base-url directly instead of through the local recording proxy")
    ap.add_argument("--run-reserve-usd", type=float,
                    help="conservative cost of one run (default: from the model's price and the token estimate)")
    ap.add_argument("--est-input-tokens", type=int, default=600_000)
    ap.add_argument("--est-output-tokens", type=int, default=40_000)
    ap.add_argument("--native-turns", type=int, default=60, help="kitsu-native: model requests per run")
    ap.add_argument("--native-tokens", type=int, default=3_000_000, help="kitsu-native: tokens per run")
    ap.add_argument("--reasoning-effort",
                    help="pin OpenCode's and Codex's reasoning effort (default: the model's default_effort from "
                         "OpenRouter's list, which is what Kitsu's loop gets by sending none)")
    ap.add_argument("--opencode-cmd", default="opencode acp")
    ap.add_argument("--codex-cmd", default="codex-acp")
    ap.add_argument("--check-fixtures", action="store_true")
    ap.add_argument("--suggest-models", action="store_true")
    ap.add_argument("--filter", help="--suggest-models: only ids matching this regex")
    args = ap.parse_args(argv)
    # The stub, the proxy and the dry-run key endpoint are on this machine.
    for k in ("NO_PROXY", "no_proxy"):
        os.environ[k] = ",".join(p for p in [os.environ.get(k, ""), "127.0.0.1", "localhost"] if p)
    if args.check_fixtures:
        return check_fixtures(load_tasks(args.tasks))
    if args.suggest_models:
        return suggest_models(args)
    return Suite(args).run()


if __name__ == "__main__":
    sys.exit(main())
