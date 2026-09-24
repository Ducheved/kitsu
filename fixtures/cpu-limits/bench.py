#!/usr/bin/env python3
"""How much agents' searches hurt the foreground, with and without limits.

Generates a synthetic tree (1M tracked lines, plus 1M lines in a gitignored
node_modules), then runs CONC agent-like bursts of 40 ripgrep searches each
while frameprobe.py does 5 ms of work every 16 ms, and prints the cores the
searches used and how long the probe's frames took.

    python3 bench.py DIR [WRAPPER...]        e.g.  nice -n 10 taskset -c 0-2
    RG=/path/to/rg CONC=3 python3 bench.py /tmp/big nice -n 10

Numbers behind `agents::Limits` (4 cores, CONC=3, three repeats):
unlimited p99 24-28 ms; nice 10: 9-14 ms, same wall time; nice 10 on
3 of 4 cores: 6-9 ms, +10% wall time.
"""
import os, random, resource, signal, subprocess, sys, threading, time

ROOT = sys.argv[1]
prefix = sys.argv[2:]
RG = os.environ.get("RG", "rg")
PATTERNS = ["zzqq", "charge_1[0-9]{3}_4", "retry.*ledger", "session_99", "(?i)HANDLER_2"]


def generate(root):
    random.seed(1)
    words = ["charge", "retry", "upstream", "payment", "ledger", "account", "token", "queue",
             "worker", "session", "index", "render", "parse", "config", "handler"]
    for top in ("src", "node_modules"):
        for i in range(20000):
            d = os.path.join(root, top, f"pkg{i % 200}", f"mod{i % 13}")
            os.makedirs(d, exist_ok=True)
            with open(os.path.join(d, f"f{i}.ts"), "w") as f:
                for j in range(50):
                    f.write(f"export function {random.choice(words)}_{i}_{j}(x: number) "
                            f"{{ return x + {j}; }} // {random.choice(words)}\n")
    with open(os.path.join(root, ".gitignore"), "w") as f:
        f.write("node_modules/\n")
    subprocess.run(["git", "init", "-q"], cwd=root, check=True)


if not os.path.exists(os.path.join(ROOT, ".gitignore")):
    os.makedirs(ROOT, exist_ok=True)
    generate(ROOT)


def burst():
    for i in range(40):
        # A path, and no stdin: without one, ripgrep searches stdin when it
        # isn't a terminal.
        subprocess.run(prefix + [RG, "-n", "--hidden", "--max-columns", "500", PATTERNS[i % len(PATTERNS)], "."],
                       cwd=ROOT, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


probe = subprocess.Popen([sys.executable, os.path.join(os.path.dirname(os.path.abspath(__file__)), "frameprobe.py")],
                         stdout=subprocess.PIPE, text=True)
time.sleep(0.3)
r0 = resource.getrusage(resource.RUSAGE_CHILDREN)
w0 = time.perf_counter()
workers = [threading.Thread(target=burst) for _ in range(int(os.environ.get("CONC", "1")))]
for t in workers:
    t.start()
for t in workers:
    t.join()
wall = time.perf_counter() - w0
r1 = resource.getrusage(resource.RUSAGE_CHILDREN)
probe.send_signal(signal.SIGTERM)
frames = probe.communicate()[0].strip()
cpu = (r1.ru_utime - r0.ru_utime) + (r1.ru_stime - r0.ru_stime)
label = " ".join(prefix) or "unlimited"
print(f"{os.environ.get('CONC', '1')}x {label:28s} wall {wall:5.2f}s cores {cpu / wall:4.2f}  {frames}", flush=True)
