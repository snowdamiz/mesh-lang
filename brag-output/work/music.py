"""Score for the Mesh brag video, composed to picture: A minor, 120 BPM, 24.0s.

Music and effects share one key and one reverb so the effects sit inside the track.
Writes music.wav (48 kHz, stereo, 16-bit). Loudness is normalised afterwards by ffmpeg.
"""
import json
import wave
import numpy as np

SR = 48000
DUR = 24.0
N = int(SR * DUR)
rng = np.random.default_rng(7)
timing = json.load(open("timing.json"))

BEAT = 0.5
S16 = BEAT / 4


def f(m):
    return 440.0 * 2 ** ((m - 69) / 12)


def tt(d):
    return np.arange(int(d * SR)) / SR


def stereo(x, pan=0.0):
    a = (pan + 1) * np.pi / 4
    return np.stack([x * np.cos(a), x * np.sin(a)]) * np.sqrt(2)


class Bus:
    def __init__(self):
        self.x = np.zeros((2, N))

    def add(self, sig, at, pan=0.0, gain=1.0):
        s = stereo(sig, pan) if sig.ndim == 1 else sig
        i = int(round(at * SR))
        if i >= N:
            return
        j0 = max(0, -i)
        i = max(0, i)
        n = min(s.shape[1] - j0, N - i)
        if n > 0:
            self.x[:, i:i + n] += gain * s[:, j0:j0 + n]


def lp_weight(freq, fc, order=2):
    return 1 / np.sqrt(1 + (freq / fc) ** (2 * order))


def saw(freq, d, fc=20000, detune_cents=0.0, phase=None):
    """Band-limited saw by additive synthesis, pre-filtered by a lowpass curve."""
    t = tt(d)
    fr = freq * 2 ** (detune_cents / 1200)
    k_max = int(min(SR * 0.45 / fr, 4 * fc / fr + 1, 120))
    ph = rng.uniform(0, 2 * np.pi) if phase is None else phase
    y = np.zeros_like(t)
    for k in range(1, max(k_max, 1) + 1):
        w = lp_weight(k * fr, fc) / k
        if w < 1e-4:
            break
        y += w * np.sin(2 * np.pi * k * fr * t + k * ph)
    return y


def square(freq, d, fc=20000):
    t = tt(d)
    y = np.zeros_like(t)
    for k in range(1, int(min(SR * 0.45 / freq, 60)) + 1, 2):
        w = lp_weight(k * freq, fc) / k
        if w < 1e-4:
            break
        y += w * np.sin(2 * np.pi * k * freq * t)
    return y


def env_adsr(d, a=0.005, dcy=0.1, s=0.6, r=0.1):
    t = tt(d)
    e = np.where(t < a, t / a, s + (1 - s) * np.exp(-(t - a) / max(dcy, 1e-4)))
    rel = np.clip((d - t) / r, 0, 1)
    return e * rel


def fft_filter(x, fn):
    """Static filter in the frequency domain; fn maps frequencies to gain."""
    X = np.fft.rfft(x)
    fr = np.fft.rfftfreq(len(x), 1 / SR)
    return np.fft.irfft(X * fn(fr), len(x))


def stft_filter(x, gain_at):
    """Time-varying filter: gain_at(time, freqs) -> gains. x is (2, n) or (n,)."""
    mono = x.ndim == 1
    xs = x[None] if mono else x
    nfft, hop = 2048, 512
    win = np.hanning(nfft + 1)[:-1]
    pad = np.pad(xs, ((0, 0), (nfft, nfft)))
    out = np.zeros_like(pad)
    fr = np.fft.rfftfreq(nfft, 1 / SR)
    for start in range(0, pad.shape[1] - nfft, hop):
        tc = (start + nfft / 2 - nfft) / SR
        g = gain_at(tc, fr)
        seg = pad[:, start:start + nfft] * win
        out[:, start:start + nfft] += np.fft.irfft(np.fft.rfft(seg, axis=1) * g, nfft, axis=1)
    out = out[:, nfft:nfft + xs.shape[1]] / 2.0  # hann at hop N/4 sums to 2
    return out[0] if mono else out


def reverb(x, rt=2.0, pre=0.018, damp=4500):
    n_ir = int(rt * 1.4 * SR)
    t = np.arange(n_ir) / SR
    irs = []
    for _ in range(2):
        noise = rng.standard_normal(n_ir) * np.exp(-6.9 * t / rt)
        noise = fft_filter(noise, lambda fr: lp_weight(fr, damp, 1) * (1 - np.exp(-fr / 180)))
        irs.append(np.concatenate([np.zeros(int(pre * SR)), noise]))
    out = np.zeros_like(x)
    L = x.shape[1] + len(irs[0])
    nfft = 1 << (L - 1).bit_length()
    for ch in range(2):
        y = np.fft.irfft(np.fft.rfft(x[ch], nfft) * np.fft.rfft(irs[ch], nfft), nfft)[: x.shape[1]]
        out[ch] = y
    return out / np.max(np.abs(out) + 1e-9) * np.max(np.abs(x) + 1e-9) * 0.9


# ───────────── harmony ─────────────
# bar i covers [2i, 2i+2)
CHORDS = {
    0: ("Am", [57, 60, 64], 33), 1: ("Am", [57, 60, 64], 33), 2: ("F", [57, 60, 65], 29),
    3: ("C", [55, 60, 64], 36), 4: ("G", [55, 59, 62], 31), 5: ("Am", [57, 60, 64], 33),
    6: ("F", [57, 60, 65], 29), 7: ("C", [55, 60, 64], 36), 8: ("G", [55, 59, 62], 31),
    9: ("E", [56, 59, 64], 28), 10: ("Am", [57, 60, 64], 33), 11: ("Am", [57, 60, 64], 33),
}
T_DROP, T_LOGO, T_ERR, T_CLICK, T_DOWN, T_REBAL, T_BACK = 2.0, 4.0, 9.0, 11.92, 12.0, 13.0, 14.5
T_ROWS, T_STAT, T_OUTRO, T_DRUMS_OUT = [15.35, 15.5, 15.65, 15.8], 16.0, 20.0, 22.0
CUTS = [3.5, 7.0, 11.0, 15.0, 18.0]

pad, arp, bass, drums, sfx, verb_send = Bus(), Bus(), Bus(), Bus(), Bus(), Bus()

# pad: three detuned saws per chord tone, slow attack, voices spread in stereo
for bar, (_, notes, _) in CHORDS.items():
    d = 2.6
    for m in notes:
        for det, pan in ((-9, -0.6), (0, 0.0), (8, 0.6)):
            v = saw(f(m), d, fc=2400, detune_cents=det) * env_adsr(d, a=0.35, dcy=1.0, s=0.8, r=0.6)
            pad.add(v, bar * 2.0 - 0.05, pan=pan, gain=0.07)

# sub + mid bass
for bar, (_, _, root) in CHORDS.items():
    if bar == 0:
        sub = np.sin(2 * np.pi * f(root) * tt(2.2)) * env_adsr(2.2, a=0.8, dcy=1, s=1, r=0.3)
        bass.add(sub, 0.0, gain=0.07)
        continue
    sub = np.sin(2 * np.pi * f(root) * tt(2.05)) * env_adsr(2.05, a=0.01, dcy=1, s=1, r=0.05)
    bass.add(sub, bar * 2.0, gain=0.1 if bar < 11 else 0.075)
    if bar >= 11:
        continue
    for k in range(8):  # driving eighths
        d = 0.24
        v = (saw(f(root + 12), d, fc=520) * 0.8 + square(f(root + 12), d, fc=380) * 0.4) * env_adsr(d, a=0.004, dcy=0.09, s=0.45, r=0.04)
        bass.add(v, bar * 2.0 + k * BEAT / 2, gain=0.3 if k % 2 else 0.22)

# arp: sixteenths over the chord, two octaves up, with a ping-pong dotted-eighth delay
PATTERN = [0, 1, 2, 3, 2, 1, 3, 4]
for bar in range(1, 12):
    _, notes, _ = CHORDS[bar]
    tones = [notes[0] + 12, notes[1] + 12, notes[2] + 12, notes[0] + 24, notes[1] + 24]
    for s in range(16):
        at = bar * 2.0 + s * S16
        if at >= 23.0:
            break
        m = tones[PATTERN[s % 8]]
        d = 0.22
        v = (square(f(m), d, fc=4200) * 0.6 + saw(f(m), d, fc=5200) * 0.4) * env_adsr(d, a=0.002, dcy=0.07, s=0.0, r=0.02)
        accent = 1.0 if s % 4 == 0 else 0.72
        fade_out = 1.0 if at < 22 else max(0.0, 1 - (at - 22) / 1.0)
        arp.add(v, at, pan=0.0, gain=0.32 * accent * fade_out)
dl = int(0.375 * SR)
echo = np.zeros_like(arp.x)
src = arp.x.copy()
for tap in range(1, 5):
    g = 0.38 ** tap
    ch = tap % 2
    shifted = np.zeros(N)
    shifted[tap * dl:] = src.sum(0)[: N - tap * dl] * 0.5
    echo[ch] += shifted * g
for ch in range(2):
    arp.x[ch] += fft_filter(echo[ch], lambda fr: lp_weight(fr, 3000))

# ───────────── drums ─────────────
def kick():
    t = tt(0.5)
    fq = 52 + 100 * np.exp(-t / 0.028)
    body = np.sin(2 * np.pi * np.cumsum(fq) / SR) * np.exp(-t / 0.16)
    click = fft_filter(rng.standard_normal(len(t)), lambda fr: np.clip(fr / 3000, 0, 1)) * np.exp(-t / 0.002) * 0.25
    return np.tanh(1.6 * (body + click)) / np.tanh(1.6)


def clap():
    t = tt(0.4)
    n = rng.standard_normal(len(t))
    e = sum(np.where(t >= o, np.exp(-(t - o) / 0.007), 0) for o in (0, 0.011, 0.022)) * 0.6 + np.where(t >= 0.022, np.exp(-(t - 0.022) / 0.11), 0)
    return fft_filter(n * e, lambda fr: np.exp(-((np.log(fr + 1) - np.log(1600)) ** 2) / 0.9))


def hat(decay):
    t = tt(decay * 5)
    return fft_filter(rng.standard_normal(len(t)), lambda fr: np.clip((fr - 4500) / 3000, 0, 1) * lp_weight(fr, 9000)) * np.exp(-t / decay)


K, C = kick(), clap()
kick_times = [b * BEAT for b in range(int(T_DROP / BEAT), int(T_DRUMS_OUT / BEAT) + 1)]
for kt in kick_times:
    drums.add(K, kt, gain=0.58)
for b in range(int(4.0 / BEAT), int(T_DRUMS_OUT / BEAT)):
    if b % 2 == 1:
        drums.add(C, b * BEAT, pan=0.05, gain=0.26)
# intro ticks: the lone server's heartbeat
for s in range(0, 16):
    drums.add(hat(0.02), s * S16, pan=0.3, gain=0.05 * (0.5 + s / 32) * (1.0 if s % 2 == 0 else 0.6))
for s in range(int(T_DROP / S16), int(T_DRUMS_OUT / S16)):
    at = s * S16
    if s % 4 == 2:
        drums.add(hat(0.075), at, pan=-0.2, gain=0.15)
    else:
        drums.add(hat(0.022), at, pan=0.25, gain=0.058 * (1.0 if s % 2 == 0 else 0.7))
# snare roll into the outro
for s in range(int(19.0 / S16), int(T_OUTRO / S16)):
    k = (s * S16 - 19.0) / 1.0
    drums.add(C, s * S16, pan=0.0, gain=0.04 + 0.2 * k ** 2)

# sidechain: bass, pad and arp breathe with the kick
t_all = np.arange(N) / SR
duck = np.ones(N)
for kt in kick_times:
    i = int(kt * SR)
    seg = t_all[i:i + int(0.4 * SR)] - kt
    duck[i:i + len(seg)] = np.minimum(duck[i:i + len(seg)], 1 - 0.55 * np.exp(-seg / 0.09))
bass.x *= duck
pad.x *= 1 - (1 - duck) * 0.6
arp.x *= 1 - (1 - duck) * 0.35


# ───────────── effects, all in A minor ─────────────
def bell(m, d=2.0, bright=3.0, ratio=2.0):
    t = tt(d)
    fc = f(m)
    idx = bright * np.exp(-t / 0.22) + 0.25
    y = np.sin(2 * np.pi * fc * t + idx * np.sin(2 * np.pi * fc * ratio * t))
    y += 0.12 * np.sin(2 * np.pi * fc * 3.51 * t) * np.exp(-t / 0.3)
    return y * np.exp(-t / (d / 3.2)) * np.clip(t / 0.002, 0, 1)


def boom(d=1.6, f0=72, f1=40):
    t = tt(d)
    fq = f1 + (f0 - f1) * np.exp(-t / 0.18)
    y = np.sin(2 * np.pi * np.cumsum(fq) / SR) * np.exp(-t / 0.4)
    n = fft_filter(rng.standard_normal(len(t)), lambda fr: lp_weight(fr, 900)) * np.exp(-t / 0.12) * 0.5
    return np.tanh(1.3 * (y + n))


def noise_sweep(d, f0, f1, shape):
    """Band of noise whose centre glides from f0 to f1; shape(k) is the amplitude over 0..1."""
    x = rng.standard_normal(int(d * SR))
    y = stft_filter(x, lambda tc, fr: np.exp(-((np.log(fr + 1) - np.log(f0 * (f1 / f0) ** np.clip(tc / d, 0, 1))) ** 2) / 0.35))
    k = np.arange(len(y)) / len(y)
    return y * shape(k)


def whoosh(at, gain=0.09, d=0.42):
    w = noise_sweep(d, 900, 3800, lambda k: np.sin(np.pi * k) ** 2)
    pans = np.linspace(-0.6, 0.6, len(w))
    a = (pans + 1) * np.pi / 4
    st = np.stack([w * np.cos(a), w * np.sin(a)]) * np.sqrt(2)
    sfx.add(st, at - d * 0.6, gain=gain)
    verb_send.add(st, at - d * 0.6, gain=gain * 0.8)


def tick(freq=3500, d=0.012, noise=0.6):
    t = tt(d)
    y = np.sin(2 * np.pi * freq * t) * (1 - noise) + rng.standard_normal(len(t)) * noise
    return fft_filter(y * np.exp(-t / (d / 4)), lambda fr: np.clip(fr / 1200, 0, 1))


def place(sig, at, pan=0.0, gain=1.0, send=0.3):
    sfx.add(sig, at, pan=pan, gain=gain)
    verb_send.add(sig, at, pan=pan, gain=gain * send)


# intro riser into the drop, and the first line's entrance
place(noise_sweep(1.9, 300, 7000, lambda k: k ** 2.2 * (1 - np.clip((k - 0.97) / 0.03, 0, 1))), 0.1, gain=0.10, send=0.4)
whoosh(0.35, gain=0.05)
place(boom(1.4), T_DROP, gain=0.5, send=0.35)
for m in (57, 64, 69):  # the fleet ships: an open fifth over A
    place(bell(m + 12, 1.8, bright=2.0), T_DROP, pan=(m - 64) / 12, gain=0.05, send=0.7)

# reversed F chord swelling into the logo, then the logo chord
rev = sum(bell(m + 12, 1.3, bright=1.5) for m in (57, 60, 65))[::-1] * np.linspace(0, 1, int(1.3 * SR)) ** 2
place(rev, T_LOGO - 1.3, gain=0.05, send=0.6)
for i, m in enumerate((65, 69, 72, 77)):
    place(bell(m, 2.4, bright=3.2), T_LOGO + i * 0.012, pan=(i - 1.5) * 0.25, gain=0.07, send=0.6)
place(boom(1.2, 80, 40), T_LOGO, gain=0.42, send=0.25)

for c in CUTS:
    whoosh(c, gain=0.07)

# typing, then the compiler says no
for i, kt in enumerate(timing["type"]):
    k = tick(freq=rng.uniform(1800, 2600), d=0.02, noise=0.8) + 0.3 * np.sin(2 * np.pi * 180 * tt(0.02)) * np.exp(-tt(0.02) / 0.004)
    place(k, kt, pan=rng.uniform(-0.15, 0.15), gain=0.22 * rng.uniform(0.7, 1.0), send=0.1)
err = (saw(f(33), 0.6, fc=700) + saw(f(34), 0.6, fc=700) * 0.9 + saw(f(45), 0.6, fc=900) * 0.5)
err = np.tanh(2.2 * err) * env_adsr(0.6, a=0.003, dcy=0.18, s=0.25, r=0.2)
place(err, T_ERR, gain=0.17, send=0.25)
place(boom(0.8, 60, 35), T_ERR, gain=0.3, send=0.1)

# cursor click, the node dies, traffic rebalances, the node comes back
place(tick(freq=1500, d=0.018, noise=0.5), T_CLICK, pan=0.35, gain=0.36, send=0.1)
t_g = tt(0.34)
glitch_f = 620 * (60 / 620) ** (t_g / 0.34)
g = np.sign(np.sin(2 * np.pi * np.cumsum(glitch_f) / SR))
g = np.repeat(g[::10], 10)[: len(t_g)]
g = np.round(g * np.exp(-t_g / 0.2) * 6) / 6
place(fft_filter(g, lambda fr: lp_weight(fr, 3000)), T_DOWN, gain=0.16, send=0.2)
for i in range(3):
    place(fft_filter(rng.standard_normal(int(0.025 * SR)), lambda fr: np.clip(fr / 2000, 0, 1)) * 0.5, T_DOWN + 0.06 * i, pan=(-0.4, 0.4, 0)[i], gain=0.1, send=0.1)
place(bell(81, 1.2, bright=1.2), T_REBAL, pan=-0.2, gain=0.09, send=0.6)
place(bell(84, 1.2, bright=1.2), T_REBAL + 0.09, pan=0.2, gain=0.08, send=0.6)
place(bell(79, 1.0, bright=1.0), T_BACK, pan=0.2, gain=0.06, send=0.6)
place(bell(84, 1.0, bright=1.0), T_BACK + 0.09, pan=-0.2, gain=0.05, send=0.6)

# benchmark rows land on C major, the stat on G
for i, (at, m) in enumerate(zip(T_ROWS, (72, 76, 79, 84))):
    place(bell(m, 0.9, bright=1.6 if i != 2 else 2.6), at, pan=(i - 1.5) * 0.3, gain=0.07 if i != 2 else 0.11, send=0.5)
place(bell(79, 1.8, bright=2.2), T_STAT, gain=0.09, send=0.6)
place(bell(86, 1.8, bright=2.2), T_STAT + 0.01, gain=0.065, send=0.6)

# tab switches
for i, at in enumerate(timing["tabs"]):
    place(tick(freq=4200, d=0.01, noise=0.3), at, pan=-0.3 + i * 0.12, gain=0.24, send=0.15)

# outro: riser, the hit, the install command typing
place(noise_sweep(1.5, 400, 8000, lambda k: k ** 2.5 * (1 - np.clip((k - 0.97) / 0.03, 0, 1))), T_OUTRO - 1.5, gain=0.1, send=0.4)
place(boom(2.2, 75, 34), T_OUTRO, gain=0.55, send=0.35)
for i, m in enumerate((69, 72, 76, 81)):
    place(bell(m, 3.2, bright=3.0), T_OUTRO + i * 0.015, pan=(i - 1.5) * 0.3, gain=0.07, send=0.7)
crash = fft_filter(rng.standard_normal(int(2.5 * SR)), lambda fr: np.clip((fr - 3000) / 4000, 0, 1) * lp_weight(fr, 12000)) * np.exp(-tt(2.5) / 0.7)
place(crash, T_OUTRO, gain=0.05, send=0.5)
for i, at in enumerate(timing["cmd"][::3]):
    place(tick(freq=rng.uniform(1800, 2600), d=0.018, noise=0.8), at, pan=rng.uniform(-0.15, 0.15), gain=0.14, send=0.1)

# ───────────── mix ─────────────
music = pad.x * 1.0 + arp.x * 1.0 + bass.x * 1.0 + drums.x * 1.0


# the intro opens up into the drop; the cluster losing a node muffles the track until it rebalances
def music_filter(tc, fr):
    if tc < T_DROP:
        fc = 500 * (16000 / 500) ** ((max(tc, 0) / T_DROP) ** 2.5)
    elif T_DOWN <= tc < T_REBAL + 0.3:
        k = (tc - T_DOWN)
        fc = 700 if k < 0.7 else 700 * (18000 / 700) ** ((k - 0.7) / 0.6)
    else:
        return np.ones_like(fr)
    return lp_weight(fr, fc, 2)


music = stft_filter(music, music_filter)
# a short dip under the compiler error
dip = np.ones(N)
i = int(T_ERR * SR)
seg = t_all[i:i + int(0.6 * SR)] - T_ERR
dip[i:i + len(seg)] = 1 - 0.5 * np.exp(-seg / 0.25)
music *= dip

verb_send.add(pad.x * 0.35 + arp.x * 0.45, 0.0)
wet = reverb(verb_send.x, rt=2.2)
mix = music + sfx.x + wet * 0.55

# master: gentle glue and a fade on the last bar
fade = np.ones(N)
fs = int(22.6 * SR)
fade[fs:] = np.cos(np.linspace(0, np.pi / 2, N - fs)) ** 1.5
mix *= fade
# high-pass at 28 Hz plus a gentle tilt (about +2 dB at 4 kHz, -5 dB at 60 Hz)
mix = np.stack([fft_filter(ch, lambda fr: 1 / np.sqrt(1 + (28 / (fr + 1e-3)) ** 4) * np.clip((fr + 1) / 1000, 0.03, 30) ** 0.2) for ch in mix])

def db(x):
    return 20 * np.log10(np.sqrt(np.mean(x ** 2)) + 1e-12)

for name, x in (("pad", pad.x), ("arp", arp.x), ("bass", bass.x), ("drums", drums.x), ("sfx", sfx.x), ("wet", wet * 0.55), ("mix", mix)):
    print(f"{name:6s} rms {db(x):6.1f} dB  peak {20 * np.log10(np.max(np.abs(x)) + 1e-12):6.1f} dB")

# gentle glue: bring peaks to -1 dB, soften only the top few dB
mix /= np.max(np.abs(mix)) / 0.89
mix = np.tanh(mix * 1.15) / np.tanh(1.15) * 0.89

pcm = (np.clip(mix.T, -1, 1) * 32767).astype("<i2")
with wave.open("music.wav", "wb") as w:
    w.setnchannels(2)
    w.setsampwidth(2)
    w.setframerate(SR)
    w.writeframes(pcm.tobytes())
print("peak", float(np.max(np.abs(mix))), "rms", float(np.sqrt(np.mean(mix ** 2))))

if __import__("sys").argv[1:] == ["--report"]:
    # effects vs music around each cue (short-term RMS, dB)
    for name, at in (("drop", T_DROP), ("logo", T_LOGO), ("typing", 8.3), ("error", T_ERR), ("click", T_CLICK), ("down", T_DOWN),
                     ("rebal", T_REBAL), ("rows", 15.6), ("stat", T_STAT), ("tabs", 19.0), ("outro", T_OUTRO), ("cmd", 21.3)):
        a, b = int((at - 0.05) * SR), int((at + 0.3) * SR)
        print(f"{name:7s} sfx {db(sfx.x[:, a:b]):6.1f}  music {db(music[:, a:b]):6.1f}")
