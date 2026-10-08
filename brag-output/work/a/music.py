"""Score for video A: the cluster, sonified. 120 BPM grid, D minor resolving to D major, 22.0s.

The melody is the traffic: every sixteenth that carries message arrivals plays a pluck whose pitch
comes from where the message landed; each spawn generation is a step in a rising arpeggio.
"""
import json
import sys
import numpy as np

sys.path.insert(0, "..")
from synth import (SR, Bus, bands, bell, boom, click, db, env, f, fft_filter, hp_weight, lp_weight, noise_sweep,
                   ping_pong, pluck, reverb, saw, stft_filter, tape_stop, tt, write_wav)

DUR = 22.0
N = int(DUR * SR)
ev = json.load(open("events.json"))
T = ev["T"]
rng = np.random.default_rng(11)
S16 = 0.125

music, plucks, drums, sfx, send = (Bus(DUR) for _ in range(5))

# harmony: (start, chord name, sub root, pad voicing, pluck scale)
SECTIONS = [
    (0.0, "Dm9", 38, [57, 60, 64, 65], [62, 64, 65, 69, 72, 74, 76, 77, 81, 84]),
    (T["dive"], "Bbmaj7#11", 34, [62, 65, 69, 76], [62, 64, 65, 69, 70, 74, 76, 77, 81, 82]),
    (T["pull"] + 0.6, "Gm9", 31, [58, 62, 65, 69], [62, 65, 67, 69, 70, 74, 77, 79, 81, 82]),
    (T["grow"], "Fmaj9", 29, [57, 60, 64, 67], [60, 64, 65, 67, 69, 72, 76, 77, 79, 81]),
    (T["warp"], "Bbmaj9", 34, [62, 65, 69, 72], [62, 65, 69, 70, 72, 74, 77, 81, 82, 84]),
    (16.7, "C6/9", 36, [62, 64, 67, 69], [62, 64, 67, 69, 72, 74, 76, 79, 81, 84]),
    (T["logo"], "Dmaj9", 38, [57, 61, 64, 66], [62, 64, 66, 69, 73, 74, 76, 78, 81, 85]),
]


def section_at(t):
    s = SECTIONS[0]
    for sec in SECTIONS:
        if t >= sec[0]:
            s = sec
    return s


# ───── pads and sub, one per section, crossfaded ─────
for i, (t0, _, root, voicing, _) in enumerate(SECTIONS):
    t1 = SECTIONS[i + 1][0] if i + 1 < len(SECTIONS) else DUR
    if t0 < 2.4:  # the hook is only the cascade; the pad blooms with the cluster
        t0 = 2.45
    d = t1 - t0 + 0.9
    fc = {"Bbmaj7#11": 900, "Dmaj9": 3200}.get(SECTIONS[i][1], 2000)
    for j, m in enumerate(voicing):
        for det, pan in ((-7, -0.55), (6, 0.55)):
            v = saw(f(m), d, fc=fc, detune=det, rng=rng) * env(d, a=0.5, dcy=1, s=1, r=0.8)
            music.add(v, t0 - 0.1, pan=pan, gain=0.034)
        shimmer = np.sin(2 * np.pi * f(m + 12) * tt(d)) * env(d, a=1.0, dcy=1, s=1, r=0.8) * (0.6 + 0.4 * np.sin(2 * np.pi * (0.3 + 0.1 * j) * tt(d)))
        music.add(shimmer, t0 - 0.1, pan=(j - 1.5) * 0.4, gain=0.012)
    if not (T["die"] <= t0 < T["grow"]):
        sub = np.sin(2 * np.pi * f(root) * tt(d)) + 0.18 * np.sin(2 * np.pi * f(root) * 2 * tt(d))
        music.add(sub * env(d, a=0.3, dcy=1, s=1, r=0.6), t0 - 0.1, gain=0.05)
# the hook: a low D that grows under the cascade
music.add(np.sin(2 * np.pi * f(26) * tt(2.8)) * np.linspace(0, 1, int(2.8 * SR)) ** 2, 0.0, gain=0.05)


def play(m, at, gain, pan=0.0, bright=0.6, t60=1.0, d=1.3, glass=0.25):
    p = pluck(f(m), d, bright=bright, t60=t60, rng=rng)
    g = np.sin(2 * np.pi * f(m) * 2 * tt(d)) * np.exp(-tt(d) / 0.08)
    plucks.add(p + glass * g, at, pan=pan, gain=gain)


# ───── the spawn cascade: one step of a rising arpeggio per generation ─────
CASCADE = [50, 57, 62, 64, 65, 69, 72, 74, 76, 77, 81, 84, 86]
for g, m in enumerate(CASCADE):
    at = T["cascade"] + g * 0.16
    play(m, at, gain=0.16 + 0.02 * g, bright=0.35 + 0.05 * g, t60=1.6)
    for k in range(min(2 ** g, 6) - 1):  # every generation doubles: more voices, strummed
        play(CASCADE[max(0, g - 1 - k % 3)] + 12 * (k % 2), at + 0.018 * (k + 1), gain=0.05, pan=(-1) ** k * 0.5, bright=0.5, t60=0.9)
sfx.add(noise_sweep(2.0, 250, 6000, lambda k: k ** 2.4 * (1 - np.clip((k - 0.96) / 0.04, 0, 1)), rng=rng), 0.6, gain=0.08)
sfx.add(boom(2.0, 70, 36, rng=rng), 2.45, gain=0.30)
for i, m in enumerate((50, 57, 64, 65, 72)):
    sfx.add(bell(m + 12, 2.6, bright=2.0), 2.45 + i * 0.02, pan=(i - 2) * 0.3, gain=0.03)

# ───── the traffic: arrivals quantised to sixteenths become the melody ─────
arr = [a for a in ev["arrivals"] if a[1] in ("m", "seed")]
slots = {}
for t, kind, cl, y, rerouted in arr:
    slots.setdefault(round(t / S16), []).append((t, kind, cl, y, rerouted))
for s, items in sorted(slots.items()):
    at = s * S16
    if at < T["msgs"] + 0.3 or at > T["conv"] + 0.2:
        continue
    if T["die"] <= at < T["grow"]:
        continue  # the tape has stopped
    diving = T["dive"] <= at < T["pull"]
    if diving and s % 2:
        continue
    scale = section_at(at)[4]
    t, kind, cl, y, rerouted = items[-1]
    idx = int(np.clip((y + 6) / 13, 0, 0.999) * len(scale))
    m = scale[idx]
    vel = min(1.0, 0.45 + 0.12 * len(items))
    accent = 1.25 if s % 4 == 0 else 1.0
    pan = {0: 0.0, 1: -0.55, 2: 0.55}[cl] + rng.uniform(-0.15, 0.15)
    play(m, at, gain=0.085 * vel * accent * (0.55 if diving else 1), pan=pan, bright=0.3 if diving else 0.55, t60=0.8)
    if kind == "seed":
        play(m + 12, at + 0.0625, gain=0.05, pan=-pan, bright=0.8, t60=0.6)
# every arrival is also a tiny click: you hear the traffic as crackle
for t, kind, cl, y, rerouted in arr:
    if T["die"] <= t < T["grow"] or t > T["conv"] + 0.2:
        continue
    music.add(click(0.004, 2500, 9000, rng=rng), t, pan={0: 0, 1: -0.6, 2: 0.6}[cl] + rng.uniform(-0.3, 0.3), gain=0.05 * rng.uniform(0.5, 1))

# ───── the new nodes bloom: fast arpeggios, left and right ─────
for base, pan in ((T["seed"] + 0.5, -0.6), (T["seed"] + 0.55, 0.6)):
    sc = SECTIONS[2][4]
    for g in range(12):
        play(sc[g % len(sc)] + (12 if g >= len(sc) else 0), base + g * 0.075, gain=0.07, pan=pan, bright=0.7, t60=0.7)
for t, cl in ev["births"]:
    if T["grow"] <= t < T["grow"] + 1.2 and rng.random() < 0.012:
        play(int(rng.choice(SECTIONS[3][4])) + 12, t, gain=0.035, pan=rng.uniform(-0.7, 0.7), bright=0.9, t60=0.5)

# ───── drums ─────
def kick(g=1.0):
    t = tt(0.45)
    body = np.sin(2 * np.pi * np.cumsum(54 + 90 * np.exp(-t / 0.025)) / SR) * np.exp(-t / 0.13)
    return np.tanh(1.5 * body) * g


def snare():
    t = tt(0.5)
    nz = fft_filter(rng.standard_normal(len(t)), lambda fr: hp_weight(fr, 1200) * lp_weight(fr, 7000)) * np.exp(-t / 0.13)
    body = np.sin(2 * np.pi * 185 * t) * np.exp(-t / 0.06)
    return nz * 0.8 + body * 0.5


K, SN = kick(), snare()
# light half-time pulse while the cluster talks
for bar in (4.0,):
    for off in (0.0, 0.75, 2.0):
        drums.add(K, bar + off, gain=0.35)
    for off in (1.0,):
        drums.add(SN, bar + off, gain=0.12)
        send.add(SN, bar + off, gain=0.12)
for s in range(int(4.0 / S16), int(T["dive"] / S16)):
    if s % 2 == 0:
        drums.add(click(0.02, 6500, 14000, rng=rng), s * S16, pan=0.3, gain=0.05 if s % 4 else 0.08)
# heartbeat in the close-up
for at in (6.5, 7.5, 8.0):
    drums.add(fft_filter(K, lambda fr: lp_weight(fr, 200)), at, gain=0.4)
# the full groove after the pull-back, and again after the reroute
PAT_K, PAT_S = {0, 3, 6, 10, 11}, {4, 12}
for s in range(int((T["pull"] + 0.6) / S16), int(T["logo"] / S16)):
    at = s * S16
    if T["die"] <= at < T["grow"]:
        continue
    pos = (s - int((T["pull"] + 0.6) / S16)) % 16
    if pos in PAT_K:
        drums.add(K, at, gain=0.42)
    if pos in PAT_S:
        drums.add(SN, at, gain=0.17)
        send.add(SN, at, gain=0.14)
    swing = 0.018 if s % 2 else 0
    drums.add(click(0.018 if pos % 4 == 2 else 0.01, 6000, 13000, rng=rng), at + swing, pan=0.25, gain=(0.09 if pos % 4 == 2 else 0.045))
    if at > 17.4:  # thirty-seconds into the logo
        drums.add(click(0.008, 7000, 14000, rng=rng), at + 0.0625, pan=-0.25, gain=0.04 + 0.05 * (at - 17.4))
for k in range(8):  # snare roll into the logo
    drums.add(SN, T["logo"] - 0.5 + k * 0.0625, gain=0.03 + 0.02 * k)

# ───── the typed mailbox ─────
sfx.add(noise_sweep(0.9, 3000, 400, lambda k: np.sin(np.pi * k) ** 2, rng=rng), T["dive"] - 0.2, gain=0.08)  # the dive
for m_kind, note in (("5", 81), ("42", 86)):
    a = next(x for x in ev["arrivals"] if x[1] == m_kind)
    sfx.add(bell(note, 1.6, bright=1.4), a[0], gain=0.09)
    play(note - 12, a[0], gain=0.1, bright=0.7, t60=1.2)
    send.add(bell(note, 1.6, bright=1.4), a[0], gain=0.08)
t_w = tt(T["reject"] - T["bad0"])
whine = np.sin(2 * np.pi * np.cumsum(f(69) * (f(75) / f(69)) ** (t_w / t_w[-1]) * (1 + 0.006 * np.sin(2 * np.pi * 6 * t_w))) / SR)
sfx.add(whine * (t_w / t_w[-1]) ** 1.5, T["bad0"], gain=0.07)
crunch = sum(saw(f(m), 0.7, fc=1800, rng=rng) for m in (38, 39, 44, 51))
crunch = np.tanh(3 * crunch) * env(0.7, a=0.002, dcy=0.12, s=0.2, r=0.3)
sfx.add(crunch, T["reject"], gain=0.1)
sfx.add(boom(0.9, 90, 38, rng=rng), T["reject"], gain=0.21)
send.add(crunch, T["reject"], gain=0.08)

# ───── pull back: whoosh, then the drop ─────
sfx.add(noise_sweep(1.1, 400, 7000, lambda k: np.sin(np.pi * k) ** 1.5, rng=rng), T["pull"] - 0.1, gain=0.14)
sfx.add(boom(1.8, 70, 34, rng=rng), T["pull"] + 0.6, gain=0.30)

# ───── a node dies: red impact, the music tape-stops, a lone heartbeat, then the reroute ─────
sfx.add(boom(1.4, 110, 32, rng=rng), T["die"], gain=0.33)
sfx.add(np.tanh(4 * sum(saw(f(m), 0.5, fc=2500, rng=rng) for m in (40, 46))) * env(0.5, a=0.002, dcy=0.1, s=0.1, r=0.2), T["die"], gain=0.07)
for at in (T["die"] + 0.3, T["die"] + 0.75):
    sfx.add(fft_filter(K, lambda fr: lp_weight(fr, 180)), at, gain=0.45)
rev = sum(bell(m, 0.6, bright=1.0) for m in (65, 69, 72, 76))[::-1] * np.linspace(0, 1, int(0.6 * SR)) ** 2
sfx.add(rev, T["grow"] - 0.6, gain=0.05)
sfx.add(noise_sweep(0.6, 500, 5000, lambda k: k ** 2, rng=rng), T["grow"] - 0.6, gain=0.08)
sfx.add(boom(1.2, 80, 38, rng=rng), T["grow"], gain=0.24)

# ───── speed: the counter, the swarm climbing into the logo ─────
for k in range(40):  # counter ticks, fast then settling
    at = 15.7 + 1.1 * (1 - (1 - k / 40) ** 2)
    sfx.add(click(0.006, 3000, 10000, rng=rng), at, pan=-0.4, gain=0.05)
sfx.add(noise_sweep(3.2, 300, 9000, lambda k: k ** 2.2 * (1 - np.clip((k - 0.97) / 0.03, 0, 1)), rng=rng), T["logo"] - 3.2, gain=0.12)
swarm = SECTIONS[4][4] + [m + 12 for m in SECTIONS[5][4]]
for k in range(46):
    x = k / 46
    at = T["conv"] + 0.4 + (T["logo"] - T["conv"] - 0.45) * (x ** 0.7)
    play(swarm[min(len(swarm) - 1, int(x * len(swarm)))], at, gain=0.05 + 0.03 * x, pan=rng.uniform(-0.8, 0.8), bright=0.5 + 0.4 * x, t60=0.6)

# ───── the logo: everything lands on one chord ─────
sfx.add(boom(2.6, 75, 32, rng=rng), T["logo"], gain=0.36)
for i, m in enumerate((50, 57, 64, 66, 69, 73, 76, 78)):
    play(m, T["logo"] + i * 0.03, gain=0.12, pan=(i - 3.5) * 0.2, bright=0.75, t60=3.0, d=3.4)
    send.add(bell(m + 12, 3.4, bright=2.2), T["logo"] + i * 0.03, pan=(i - 3.5) * 0.2, gain=0.035)
    sfx.add(bell(m + 12, 3.4, bright=2.2), T["logo"] + i * 0.03, pan=(i - 3.5) * 0.2, gain=0.03)

# ───── mix ─────
plucks.x = np.stack([fft_filter(c, lambda fr: hp_weight(fr, 90)) for c in plucks.x])
plucks.x += ping_pong(plucks.x, 0.375, fb=0.35, taps=4) * 0.8
bus = music.x + plucks.x * 3.2 + drums.x


def filt(tc, fr):
    if tc < 2.45:  # the hook opens up as the cluster grows
        return lp_weight(fr, 700 * (12000 / 700) ** (max(tc, 0) / 2.45) ** 2, 2)
    if T["dive"] < tc < T["pull"]:  # inside the cluster: close and muffled
        k = min(1, (tc - T["dive"]) / 0.5) * min(1, (T["pull"] - tc) / 0.3)
        return lp_weight(fr, 16000 * (1 - k) + 1400 * k, 2)
    return None


bus = stft_filter(bus, filt)
bus = tape_stop(bus, T["die"], 0.35, silence_until=T["grow"] - 0.05)
dip = np.ones(N)  # a breath of silence after the rejected message
i0 = int(T["reject"] * SR)
dip[i0:i0 + int(0.5 * SR)] = 1 - 0.75 * np.exp(-np.arange(int(0.5 * SR)) / SR / 0.25)
bus *= dip

send.x += bus * 0.3 + plucks.x * 1.0 + sfx.x * 0.25
wet = reverb(send.x, rt=2.8)
mix = bus + sfx.x + wet * 0.5
mix = np.stack([fft_filter(c, lambda fr: hp_weight(fr, 28) * np.clip((fr + 1) / 1000, 0.03, 30) ** 0.12) for c in mix])
fade = np.ones(N)
fs = int((DUR - 1.2) * SR)
fade[fs:] = np.cos(np.linspace(0, np.pi / 2, N - fs)) ** 1.3
mix *= fade

for name, x in (("music", music.x), ("plucks", plucks.x), ("drums", drums.x), ("sfx", sfx.x), ("wet", wet * 0.5), ("mix", mix)):
    print(f"{name:7s} {db(x):6.1f} dB")
print(bands(mix, 3, 18))
write_wav("music.wav", mix)

if "--stems" in sys.argv:
    for name, x in (("music", music.x), ("plucks", plucks.x * 3.2), ("drums", drums.x), ("sfx", sfx.x)):
        lo = np.stack([fft_filter(c, lambda fr: lp_weight(fr, 60, 4)) for c in x])
        print(f"{name:7s} total {db(x):6.1f}  <60Hz {db(lo):6.1f}")
