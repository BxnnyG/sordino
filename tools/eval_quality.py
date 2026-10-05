#!/usr/bin/env python3
"""Objective speech-quality benchmark for the Sordino pipeline.

    python3 tools/eval_quality.py [--bin target/release/examples/process_file] [--args "..."] ...

Downloads public test material from the DeepFilterNet repository (clean speech and two noise
recordings), mixes them at several SNRs, runs them through `process_file` and reports
SI-SDR (dB), STOI and PESQ (wide band) against the clean speech. Higher is better for all three.

Each `--args` string is one variant, e.g. --args "--noise high --studio off".
Needs: numpy scipy soundfile pystoi pesq.
"""
import argparse
import os
import shlex
import subprocess
import tempfile
import urllib.request
from pathlib import Path

import numpy as np
import soundfile as sf
from pesq import pesq
from pystoi import stoi
from scipy import signal

BASE = "https://raw.githubusercontent.com/Rikorose/DeepFilterNet/d375b2d8309e0935d165700c91da9de862a99c31/assets/"
FILES = ["clean_freesound_33711.wav", "noise_freesound_2530.wav", "noise_freesound_573577.wav"]
SR = 48000


def fetch(cache: Path):
    cache.mkdir(parents=True, exist_ok=True)
    for f in FILES:
        p = cache / f
        if not p.exists():
            urllib.request.urlretrieve(BASE + f, p)
    return cache


def mono(path):
    x, sr = sf.read(path, dtype="float32")
    assert sr == SR, f"{path}: {sr} Hz"
    return x.mean(axis=1) if x.ndim > 1 else x


def conditions(cache):
    clean = mono(cache / FILES[0])
    out = {"clean": clean}
    for i, nf in enumerate(FILES[1:], 1):
        n = mono(cache / nf)
        n = np.tile(n, int(np.ceil((len(clean) + SR * 3) / len(n))))[SR * 3 : SR * 3 + len(clean)]
        for snr in (20, 10, 0):
            k = np.sqrt((clean**2).mean() / ((n**2).mean() * 10 ** (snr / 10)))
            out[f"noise{i}_snr{snr}"] = (clean + k * n).astype("float32")
    return clean, out


def align(ref, x):
    n = min(len(ref), len(x))
    a, b = ref[:n], x[:n]
    d = 8
    cc = signal.correlate(signal.resample_poly(a, 1, d), signal.resample_poly(b, 1, d), mode="full", method="fft")
    coarse = (np.argmax(cc) - (len(a) // d - 1)) * d
    best = None
    for lag in range(coarse - 16, coarse + 17):
        y = b[lag:] if lag >= 0 else np.concatenate([np.zeros(-lag, dtype=b.dtype), b])[:n]
        m = min(len(y), len(a))
        c = float(np.dot(a[:m], y[:m]))
        if best is None or c > best[0]:
            best = (c, lag)
    lag = best[1]
    y = b[lag:] if lag >= 0 else np.concatenate([np.zeros(-lag, dtype=b.dtype), b])[:n]
    m = min(len(y), len(a))
    return a[:m], y[:m]


def score(clean, x):
    r, y = align(clean, x)
    r0, y0 = r - r.mean(), y - y.mean()
    t = np.dot(y0, r0) / np.dot(r0, r0) * r0
    sisdr = 10 * np.log10(np.dot(t, t) / (np.dot(y0 - t, y0 - t) + 1e-12))
    st = stoi(r, y, SR, extended=False)
    try:
        pq = pesq(16000, signal.resample_poly(r, 1, 3), signal.resample_poly(y, 1, 3), "wb")
    except Exception:
        pq = float("nan")
    return sisdr, st, pq


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", default="target/release/examples/process_file")
    ap.add_argument("--args", action="append", default=None)
    ap.add_argument("--cache", default=os.path.expanduser("~/.cache/sordino-eval"))
    a = ap.parse_args()
    variants = a.args or ["--noise high --studio off"]
    clean, conds = conditions(fetch(Path(a.cache)))
    with tempfile.TemporaryDirectory() as tmp:
        print(f"{'condition':16s} {'input':>20s}  " + "  ".join(f"{v[:24]:>24s}" for v in variants))
        totals = {v: [] for v in variants}
        base = []
        for name, x in conds.items():
            inp = Path(tmp) / f"{name}.f32"
            x.astype("<f4").tofile(inp)
            b = score(clean, x) if name != "clean" else (float("nan"), 1.0, 4.64)
            base.append(b)
            line = f"{name:16s} {b[0]:6.1f}/{b[1]:.3f}/{b[2]:.2f}  "
            for v in variants:
                out = Path(tmp) / "out.f32"
                subprocess.run([a.bin, str(inp), str(out), *shlex.split(v)], check=True, stderr=subprocess.DEVNULL)
                s = score(clean, np.fromfile(out, dtype="<f4"))
                totals[v].append(s)
                line += f"  {s[0]:8.1f}/{s[1]:.3f}/{s[2]:.2f}"
            print(line)
        print("\nmean (SI-SDR dB / STOI / PESQ):")
        for v in variants:
            arr = np.array(totals[v])
            print(f"  {v:34s} {arr[:,0].mean():6.2f} / {arr[:,1].mean():.3f} / {arr[:,2].mean():.2f}")


if __name__ == "__main__":
    main()
