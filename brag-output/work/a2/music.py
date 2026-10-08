"""Soundtrack for video A (57.6 s): "Futuristic Pulse" by Universfield (Pixabay Content License),
edited on its bar lines to the picture, with quiet effects in B minor underneath.

Edit (track seconds -> video seconds), every cut on a measured downbeat:
  intro + first main section   0.055 - 34.611  ->  0.000 - 34.553   (drop at 14.89 = the dive)
  breakdown, first two bars   45.845 - 51.464  -> 34.553 - 40.171   (the node dies)
  build + return              57.083 - 65.509  -> 40.171 - 48.599   (return at 42.98 = the flight)
  outro                       90.792 - end     -> 48.599 - end      (the logo)
"""
import json
import subprocess
import sys
import numpy as np

sys.path.insert(0, "..")
from synth import (SR, Bus, bell, boom, click, db, env, f, fft_filter, hp_weight, lp_weight, noise_sweep, reverb,
                   saw, tt)

TRACK = "../music/universfield-futuristic-pulse-212484.mp3"
ev = json.load(open("events.json"))
T, DUR = ev["T"], ev["DUR"]
N = int(DUR * SR)
rng = np.random.default_rng(5)

raw = subprocess.run(["ffmpeg", "-v", "error", "-i", TRACK, "-ar", str(SR), "-ac", "2", "-f", "f32le", "-"], capture_output=True).stdout
track = np.frombuffer(raw, np.float32).reshape(-1, 2).T.astype(np.float64)

# onset envelope, to snap each cut to the real transient near the nominal downbeat
mono = track.mean(0)
hop, nfft = 128, 1024
frames = np.lib.stride_tricks.sliding_window_view(mono, nfft)[::hop] * np.hanning(nfft)
flux = np.maximum(0, np.diff(np.log1p(np.abs(np.fft.rfft(frames, axis=1))), axis=0)).sum(1)


def snap(t, win=0.04):
    i0, i1 = int((t - win) * SR / hop), int((t + win) * SR / hop)
    return (i0 + int(np.argmax(flux[i0:i1])) + 1) * hop / SR + nfft / 2 / SR


# The measured downbeats sit 55 ms after the coarse grid, so the track starts 55 ms in and every cut
# lands on a real transient (snapped within 25 ms); the picture's grid then matches the audio exactly.
OFF = 0.055
SEGS = [(0.0, 34.553), (45.790, 51.409), (57.027, 65.455), (90.739, track.shape[1] / SR)]
cuts = [(snap(a + OFF, 0.025) if a > 0 else OFF, snap(b + OFF, 0.025) if b < SEGS[-1][1] else b) for a, b in SEGS]
print("cuts", [(round(a, 3), round(b, 3)) for a, b in cuts])

XF = int(0.03 * SR)
out = np.zeros((2, 0))
for i, (a, b) in enumerate(cuts):
    ia, ib = int(a * SR), int(b * SR)
    seg = track[:, max(0, ia - (XF if i else 0)):ib].copy()
    if i == 0:
        out = seg
        continue
    ramp = np.sin(np.linspace(0, np.pi / 2, XF)) ** 2
    tail = out[:, -XF:] * ramp[::-1]
    head = seg[:, :XF] * ramp
    out = np.concatenate([out[:, :-XF], tail + head, seg[:, XF:]], axis=1)
music = np.zeros((2, N))
k = min(N, out.shape[1])
music[:, :k] = out[:, :k]
fade = np.ones(N)
f0 = int((DUR - 0.6) * SR)
fade[f0:] = np.cos(np.linspace(0, np.pi / 2, N - f0))
music *= fade

# ───── effects, quiet and in B minor ─────
sfx, send = Bus(DUR), Bus(DUR)


def place(sig, at, pan=0.0, gain=1.0, wet=0.4):
    sfx.add(sig, at, pan=pan, gain=gain)
    send.add(sig, at, pan=pan, gain=gain * wet)


def whoosh(at, d=0.9, gain=0.05, f0=500, f1=5000):
    place(noise_sweep(d, f0, f1, lambda k: np.sin(np.pi * k) ** 1.6, rng=rng), at - d * 0.75, gain=gain, wet=0.5)


place(bell(83, 2.2, bright=1.2), T["first"], gain=0.05, wet=0.8)                   # the first actor
place(np.sin(2 * np.pi * f(35) * tt(1.4)) * np.exp(-tt(1.4) / 0.5), T["first"], gain=0.12, wet=0.1)
place(noise_sweep(2.0, 800, 9000, lambda k: np.sin(np.pi * k) ** 2, rng=rng), T["cascade"], gain=0.035, wet=0.6)
whoosh(T["dive"], d=1.2, gain=0.06, f0=4000, f1=600)                                # the dive
for kind, note in (("m5", 78), ("m42", 83)):                                         # 5 and 42 land
    place(bell(note, 1.4, bright=1.0), T[kind] + 0.6, gain=0.045, wet=0.7)
    place(click(0.006, 2000, 9000, rng=rng), T[kind] + 0.6, gain=0.08, wet=0.1)
t_w = tt(T["reject"] - T["bad0"])                                                    # "five" closes in
whine = np.sin(2 * np.pi * np.cumsum(f(71) * (f(77) / f(71)) ** (t_w / t_w[-1])) / SR) * (t_w / t_w[-1]) ** 2
place(whine, T["bad0"], gain=0.025, wet=0.3)
crunch = np.tanh(3 * fft_filter(rng.standard_normal(int(0.35 * SR)), lambda fr: hp_weight(fr, 300) * lp_weight(fr, 3000))) * np.exp(-tt(0.35) / 0.08)
place(crunch, T["reject"], gain=0.05, wet=0.3)                                       # rejected
place(boom(0.9, 90, 42, rng=rng), T["reject"], gain=0.22, wet=0.1)
g = np.repeat(np.sign(rng.standard_normal(int(0.3 * SR) // 24)), 24)[: int(0.3 * SR)] * np.exp(-tt(0.3) / 0.1)
place(fft_filter(g, lambda fr: lp_weight(fr, 5000)), T["crash"], gain=0.035, wet=0.2)   # a child crashes
place(boom(0.6, 110, 50, rng=rng), T["crash"], gain=0.12, wet=0.1)
for i, note in enumerate((83, 90)):                                                  # restarted
    place(bell(note, 1.6, bright=1.4), T["reborn"] + i * 0.06, pan=(-0.3, 0.3)[i], gain=0.035, wet=0.8)
whoosh(T["pull"] + 0.5, d=1.6, gain=0.06, f0=300, f1=6000)                           # pull back
place(boom(1.8, 80, 34, rng=rng), T["die"], gain=0.3, wet=0.2)                        # a node dies
place(fft_filter(rng.standard_normal(int(0.5 * SR)), lambda fr: hp_weight(fr, 1500)) * np.exp(-tt(0.5) / 0.12), T["die"], gain=0.03, wet=0.4)
place(noise_sweep(1.7, 400, 8000, lambda k: k ** 2.2 * (1 - np.clip((k - 0.96) / 0.04, 0, 1)), rng=rng), T["warp"] - 1.7, gain=0.05, wet=0.5)
for k in range(30):                                                                   # the counter
    place(click(0.005, 3000, 10000, rng=rng), T["warp"] + 0.9 + 1.5 * (1 - (1 - k / 30) ** 2), pan=-0.4, gain=0.03, wet=0.05)
for i, note in enumerate((59, 66, 71, 73, 74, 78)):                                   # the logo lands
    place(bell(note + 12, 3.2, bright=1.6), T["logo"] + i * 0.03, pan=(i - 2.5) * 0.25, gain=0.022, wet=0.9)
# the traffic, as a faint crackle
for t, kind, cl, y, rerouted in ev["arrivals"]:
    if kind in ("m", "seed") and not (T["die"] <= t < T["grow"]) and t < T["conv"]:
        sfx.add(click(0.003, 3000, 10000, rng=rng), t, pan={0: 0, 1: -0.6, 2: 0.6}[cl] + rng.uniform(-0.3, 0.3), gain=0.018 * rng.uniform(0.4, 1))

wet = reverb(send.x, rt=2.4)
mix = music + sfx.x + wet * 0.45
print(f"music {db(music):.1f} dB  sfx {db(sfx.x):.1f} dB  wet {db(wet * 0.45):.1f} dB")
import wave
with wave.open("mix.wav", "wb") as w:
    w.setnchannels(2); w.setsampwidth(2); w.setframerate(SR)
    w.writeframes((np.clip(mix.T / max(1.0, np.abs(mix).max() / 0.95), -1, 1) * 32767).astype("<i2").tobytes())
