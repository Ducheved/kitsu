"""Foreground stand-in for bench.py: 5 ms of work every 16 ms; on SIGTERM
prints how long its frames took (p50, p99, max)."""
import signal, sys, time
frames = []
def done(*_):
    f = sorted(frames); p = lambda q: f[int(q * (len(f) - 1))] * 1000
    print(f"frame p50 {p(.5):5.1f}ms p99 {p(.99):5.1f}ms max {f[-1]*1000:5.1f}ms n={len(f)}", flush=True); sys.exit(0)
signal.signal(signal.SIGTERM, done)
nxt = time.perf_counter() + 0.016
while True:
    t0 = time.perf_counter()
    while time.perf_counter() - t0 < 0.005: pass
    frames.append(max(0.0, time.perf_counter() - nxt))
    nxt += 0.016
    d = nxt - time.perf_counter()
    if d > 0: time.sleep(d)
    else: nxt = time.perf_counter() + 0.016
