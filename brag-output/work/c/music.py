"""Score for video C: drum-led glitch in F minor, 120 BPM, 20.0s. Every cut in the edit is a hit."""
import json
import sys
import numpy as np

sys.path.insert(0, "..")
from synth import (SR, Bus, bands, boom, click, db, env, f, fft_filter, hp_weight, lp_weight, noise_sweep, reverb,
                   saw, stft_filter, tape_stop, tt, write_wav)

DUR = 20.0
N = int(DUR * SR)
cues = json.load(open("events.json"))
rng = np.random.default_rng(29)
S16 = 0.125

drums, bass, stabs, sfx, send = (Bus(DUR) for _ in range(5))


# ───── instruments ─────
def kick():
    t = tt(0.4)
    body = np.tanh(2.2 * np.sin(2 * np.pi * np.cumsum(50 + 120 * np.exp(-t / 0.022)) / SR) * np.exp(-t / 0.16)) * 0.9
    c = click(0.004, 1500, 9000, rng=rng)
    body[: len(c)] += c * 0.5
    return body


def snare():
    t = tt(0.4)
    nz = fft_filter(rng.standard_normal(len(t)), lambda fr: hp_weight(fr, 1500) * lp_weight(fr, 9000)) * np.exp(-t / 0.11)
    body = np.sin(2 * np.pi * 205 * t) * np.exp(-t / 0.05)
    clap = sum(np.where(t >= o, np.exp(-(t - o) / 0.006), 0) for o in (0, 0.009, 0.018)) * fft_filter(rng.standard_normal(len(t)), lambda fr: hp_weight(fr, 900) * lp_weight(fr, 5000))
    return np.tanh(1.5 * (nz * 0.9 + body * 0.6 + clap * 0.5))


def hat(d):
    return click(d, 7000, 15000, rng=rng)


def rim():
    t = tt(0.08)
    return (np.sin(2 * np.pi * 1750 * t) * 0.6 + rng.standard_normal(len(t)) * 0.3) * np.exp(-t / 0.012)


def e808(m, d, glide_from=None):
    t = tt(d)
    fr = f(m) if glide_from is None else f(m) + (f(glide_from) - f(m)) * np.exp(-t / 0.05)
    y = np.sin(2 * np.pi * np.cumsum(fr * np.ones_like(t)) / SR)
    return np.tanh(2.6 * y) * env(d, a=0.003, dcy=0.35, s=0.55, r=0.05)


def stab(notes, d=0.32, bright=4200):
    t = tt(d)
    y = np.zeros_like(t)
    for m in notes:
        for det in (-10, 10):
            fr = f(m) * 2 ** (det / 1200)
            blip = 1 + 1.0 * np.exp(-t / 0.012)  # a quick pitch drop gives it bite
            ph = 2 * np.pi * np.cumsum(fr * blip) / SR
            y += sum(np.sin(k * ph) / k for k in range(1, 12)) * 0.5
    y = fft_filter(y, lambda fr: hp_weight(fr, 250) * lp_weight(fr, bright))
    return y * env(d, a=0.002, dcy=0.09, s=0.25, r=0.08) / len(notes)


def crash(d=1.4):
    t = tt(d)
    return fft_filter(rng.standard_normal(len(t)), lambda fr: hp_weight(fr, 4000) * lp_weight(fr, 14000)) * np.exp(-t / 0.45)


def reverse_cymbal(d=0.5):
    x = crash(d)[::-1]
    return x * np.linspace(0, 1, len(x)) ** 2


K, SN, R = kick(), snare(), rim()
FM9 = [53, 56, 60, 63, 67]      # F Ab C Eb G
DB = [49, 53, 56, 60]           # Db F Ab C
EB = [51, 55, 58, 62]           # Eb G Bb D
CM = [48, 51, 55, 58]           # C Eb G Bb
PROG = [FM9, FM9, DB, EB, FM9, CM, DB, EB, FM9, FM9]  # one chord per bar (2 s)
ROOT = {id(FM9): 29, id(DB): 25, id(EB): 27, id(CM): 24}

# ───── cold open: one hit per letter, rising ─────
for i, at in enumerate((0.0, 0.25, 0.5, 0.75)):
    drums.add(K if i % 2 == 0 else SN, at, gain=0.8 if i % 2 == 0 else 0.6)
    stabs.add(stab([[53, 60], [56, 63], [60, 67], [63, 70]][i], 0.24), at, gain=0.5)
drums.add(K, 1.0, gain=0.9)
sfx.add(crash(1.6), 1.0, gain=0.18)
stabs.add(stab(FM9 + [72], 0.6, bright=6000), 1.0, gain=0.6)
bass.add(e808(29, 0.9, glide_from=41), 1.0, gain=0.5)

# ───── the groove ─────
PAT_K = {0, 6, 10}
PAT_S = {4, 12}
RIFF = {0: 0, 3: 0, 6: 3, 10: 7, 11: 10, 14: 12}  # semitones over the bar's root, on sixteenths


def groove(t0, t1, hats=True, snare=True, riff=True, dense=False):
    for s in range(int(round(t0 / S16)), int(round(t1 / S16))):
        at = s * S16
        pos = s % 16
        bar = int(at // 2)
        swing = 0.016 if s % 2 else 0.0
        if pos in PAT_K:
            drums.add(K, at, gain=0.62)
        if snare and pos in PAT_S:
            drums.add(SN, at, gain=0.5)
            send.add(SN, at, gain=0.25)
        if pos in (7, 13):
            drums.add(R, at + swing, pan=0.35, gain=0.18)
        if hats:
            if pos in (2, 10):
                drums.add(hat(0.09), at, pan=-0.25, gain=0.22)
            elif dense or s % 2 == 0:
                drums.add(hat(0.018), at + swing, pan=0.25, gain=0.13 if s % 2 == 0 else 0.08)
        if riff and pos in RIFF:
            chord = PROG[min(bar, len(PROG) - 1)]
            root = ROOT[id(chord)]
            nxt = [p for p in sorted(RIFF) if p > pos]
            d = ((nxt[0] if nxt else 16) - pos) * S16
            bass.add(e808(root + RIFF[pos], d, glide_from=root + RIFF[pos] + 5 if pos == 10 else None), at, gain=0.42)


groove(1.0, 2.5, snare=False, riff=True)
groove(2.5, 5.0)
# red card: half-time and crushed
drums.add(K, 5.0, gain=0.95); drums.add(SN, 6.0, gain=0.6); send.add(SN, 6.0, gain=0.4)
for s in range(int(5.0 / S16), int(6.5 / S16)):
    if s % 2 == 0:
        drums.add(hat(0.02), s * S16, pan=0.2, gain=0.1)
bass.add(e808(29, 1.4, glide_from=36), 5.0, gain=0.5)
groove(6.5, 8.0, dense=True)
groove(8.5, 13.0)
groove(13.0, 14.0, snare=False)
groove(16.0, 17.0, snare=False)
groove(17.0, 19.0, dense=True)

# ───── hits on every slam ─────
motif = [65, 68, 72, 75, 77, 80, 84]
for i, at in enumerate(cues["slams"]):
    if at < 1.0 or at in cues["words"]:
        continue
    stabs.add(stab(PROG[min(int(at // 2), 9)], 0.3), at, pan=(-1) ** i * 0.2, gain=0.42)
    drums.add(K, at, gain=0.5)
for at in cues["flashes"]:
    if at > 1.0:
        sfx.add(crash(1.2), at, gain=0.12)
        sfx.add(reverse_cymbal(0.5), at - 0.5, gain=0.1)

# nodes appear: three blips; then the lost node
for i, at in enumerate((6.75, 7.0, 7.25)):
    stabs.add(stab([72 + [0, 3, 7][i]], 0.2, bright=6000), at, pan=(i - 1) * 0.6, gain=0.35)
alarm = np.concatenate([np.sign(np.sin(2 * np.pi * f(m) * tt(0.12))) for m in (77, 71, 77, 71)])
sfx.add(fft_filter(alarm, lambda fr: lp_weight(fr, 3000)) * 0.5, 8.0, gain=0.1)
sfx.add(boom(1.0, 90, 38, rng=rng), 8.0, gain=0.4)
sfx.add(reverse_cymbal(0.45), 8.05, gain=0.14)

# counter: a rising run of blips; words: a stab per word climbing the scale
for k in range(24):
    at = 10.5 + 1.2 * (k / 24) ** 0.8
    m = motif[k % len(motif)] + 12 * (k // len(motif))
    stabs.add(np.sin(2 * np.pi * f(m) * tt(0.06)) * np.exp(-tt(0.06) / 0.02), at, pan=(k % 3 - 1) * 0.4, gain=0.12)
for i, at in enumerate(cues["words"]):
    drums.add(K if i % 2 == 0 else SN, at, gain=0.7)
    stabs.add(stab([motif[i], motif[i] + 7], 0.22, bright=5000), at, pan=(-1) ** i * 0.3, gain=0.45)
for k in range(8):  # snare roll into the outro
    drums.add(SN, 16.0 + 0.5 + k * 0.0625, gain=0.12 + 0.05 * k)
sfx.add(noise_sweep(1.0, 500, 9000, lambda k: k ** 2.3, rng=rng), 16.0, gain=0.14)
sfx.add(noise_sweep(0.5, 800, 6000, lambda k: np.sin(np.pi * k), rng=rng), 12.9, gain=0.1)

# the outro hit and the final chord
sfx.add(boom(2.0, 80, 36, rng=rng), 17.0, gain=0.45)
sfx.add(crash(2.4), 17.0, gain=0.2)
stabs.add(stab(FM9 + [72, 75], 0.9, bright=7000), 17.0, gain=0.55)
stabs.add(stab(FM9 + [72], 1.6, bright=5000), 19.0, gain=0.5)
drums.add(K, 19.0, gain=0.9)
sfx.add(crash(2.0), 19.0, gain=0.16)
bass.add(e808(29, 1.0, glide_from=41), 19.0, gain=0.5)

# ───── glitch: stutter repeats after the big slams, crushed drums on the red card ─────
# a dark chord bed under the groove, one chord per bar
pad = Bus(DUR)
for bar, chord in enumerate(PROG):
    t0 = bar * 2.0
    if t0 < 2.0:
        continue
    for j, m in enumerate(chord):
        for det, pn in ((-8, -0.5), (8, 0.5)):
            pad.add(saw(f(m + 12), 2.2, fc=1600, detune=det, rng=rng) * env(2.2, a=0.08, dcy=1, s=1, r=0.3), t0, pan=pn, gain=0.03)
bus = drums.x + bass.x * 0.75 + stabs.x * 3.0 + pad.x
for at in (1.0, 5.0, 8.5, 13.0, 15.75, 17.0):
    i, w = int(at * SR), int(S16 / 2 * SR)
    seg = bus[:, i:i + w].copy()
    for k in range(1, 4):
        j = i + k * w
        bus[:, j:j + w] = bus[:, j:j + w] * 0.3 + seg * 0.75 ** k
i0, i1 = int(5.0 * SR), int(6.5 * SR)
crushed = np.round(bus[:, i0:i1] * 12) / 12
crushed = np.repeat(crushed[:, ::5], 5, axis=1)[:, : i1 - i0]
bus[:, i0:i1] = crushed * 0.85
bus = tape_stop(bus, 8.0, 0.28, silence_until=8.45)
bus = stft_filter(bus, lambda tc, fr: lp_weight(fr, 500 * (16000 / 500) ** ((tc - 1.0) / 1.5) ** 2, 2) if 1.05 < tc < 2.5 else None)

send.x += stabs.x * 0.4 + sfx.x * 0.2
wet = reverb(send.x, rt=1.6, damp=6000)
mix = bus + sfx.x + wet * 0.35
mix = np.stack([fft_filter(c, lambda fr: hp_weight(fr, 30)) for c in mix])
fade = np.ones(N)
fs = int((DUR - 0.9) * SR)
fade[fs:] = np.cos(np.linspace(0, np.pi / 2, N - fs))
mix *= fade

for name, x in (("drums", drums.x), ("bass", bass.x * 0.75), ("stabs", stabs.x * 3), ("pad", pad.x), ("sfx", sfx.x), ("wet", wet * 0.35), ("mix", mix)):
    print(f"{name:6s} {db(x):6.1f} dB")
print(bands(mix, 2.5, 17))
write_wav("music.wav", mix)
