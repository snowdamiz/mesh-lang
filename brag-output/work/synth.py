"""Small numpy synth kit shared by both scores."""
import wave
import numpy as np

SR = 48000


def f(m):
    return 440.0 * 2 ** ((m - 69) / 12)


def tt(d):
    return np.arange(int(d * SR)) / SR


def stereo(x, pan=0.0):
    a = (np.clip(pan, -1, 1) + 1) * np.pi / 4
    return np.stack([x * np.cos(a), x * np.sin(a)]) * np.sqrt(2)


class Bus:
    def __init__(self, dur):
        self.n = int(dur * SR)
        self.x = np.zeros((2, self.n))

    def add(self, sig, at, pan=0.0, gain=1.0):
        s = stereo(sig, pan) if sig.ndim == 1 else sig
        i = int(round(at * SR))
        if i >= self.n:
            return
        j0 = max(0, -i)
        i = max(0, i)
        k = min(s.shape[1] - j0, self.n - i)
        if k > 0:
            self.x[:, i:i + k] += gain * s[:, j0:j0 + k]


def lp_weight(freq, fc, order=2):
    return 1 / np.sqrt(1 + (freq / fc) ** (2 * order))


def hp_weight(freq, fc, order=2):
    return 1 / np.sqrt(1 + (fc / (freq + 1e-6)) ** (2 * order))


def fft_filter(x, fn):
    X = np.fft.rfft(x)
    fr = np.fft.rfftfreq(len(x), 1 / SR)
    return np.fft.irfft(X * fn(fr), len(x))


def stft_filter(x, gain_at):
    """Time-varying filter; gain_at(time, freqs) -> gains. x is (n,) or (2, n)."""
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
        if g is None:
            out[:, start:start + nfft] += pad[:, start:start + nfft] * win
            continue
        seg = pad[:, start:start + nfft] * win
        out[:, start:start + nfft] += np.fft.irfft(np.fft.rfft(seg, axis=1) * g, nfft, axis=1)
    out = out[:, nfft:nfft + xs.shape[1]] / 2.0
    return out[0] if mono else out


def env(d, a=0.005, dcy=0.1, s=0.6, r=0.1):
    t = tt(d)
    e = np.where(t < a, t / max(a, 1e-4), s + (1 - s) * np.exp(-(t - a) / max(dcy, 1e-4)))
    return e * np.clip((d - t) / max(r, 1e-4), 0, 1)


def saw(freq, d, fc=20000, detune=0.0, rng=None):
    t = tt(d)
    fr = freq * 2 ** (detune / 1200)
    ph = (rng.uniform(0, 2 * np.pi) if rng is not None else 0.0)
    y = np.zeros_like(t)
    for k in range(1, int(min(SR * 0.45 / fr, 4 * fc / fr + 1, 120)) + 1):
        w = lp_weight(k * fr, fc) / k
        if w < 1e-4:
            break
        y += w * np.sin(2 * np.pi * k * fr * t + k * ph)
    return y


def pluck(freq, d, bright=0.6, t60=1.2, rng=None):
    """Karplus-Strong string, generated at an integer period and resampled to pitch."""
    rng = rng or np.random.default_rng(0)
    per = max(2, int(SR / freq))
    f_gen = SR / per
    ratio = freq / f_gen
    n = int(d * SR)
    L = int(n * ratio) + per + 2
    burst = rng.uniform(-1, 1, per)
    burst = fft_filter(burst, lambda fr: lp_weight(fr, 800 + 9000 * bright, 1))
    y = np.zeros(L)
    y[:per] = burst
    g = 10 ** (-3 * per / (SR * t60))
    for s in range(per, L, per):
        e = min(L, s + per)
        a = y[s - per:e - per]
        b = np.concatenate(([y[s - per - 1]] if s - per - 1 >= 0 else [0.0], y[s - per:e - per - 1]))
        y[s:e] = g * 0.5 * (a + b[: e - s])
    out = np.interp(np.arange(n) * ratio, np.arange(L), y)
    out *= np.clip(np.arange(n) / (0.002 * SR), 0, 1) * np.clip((n - np.arange(n)) / (0.02 * SR), 0, 1)
    return out / (np.max(np.abs(out)) + 1e-9)


def bell(m, d=2.0, bright=3.0, ratio=2.0):
    t = tt(d)
    fc = f(m)
    idx = bright * np.exp(-t / 0.22) + 0.25
    y = np.sin(2 * np.pi * fc * t + idx * np.sin(2 * np.pi * fc * ratio * t))
    y += 0.1 * np.sin(2 * np.pi * fc * 3.51 * t) * np.exp(-t / 0.3)
    return y * np.exp(-t / (d / 3.2)) * np.clip(t / 0.002, 0, 1)


def boom(d=1.6, f0=72, f1=40, rng=None):
    rng = rng or np.random.default_rng(1)
    t = tt(d)
    fq = f1 + (f0 - f1) * np.exp(-t / 0.18)
    y = np.sin(2 * np.pi * np.cumsum(fq) / SR) * np.exp(-t / 0.45)
    nz = fft_filter(rng.standard_normal(len(t)), lambda fr: lp_weight(fr, 900)) * np.exp(-t / 0.1) * 0.4
    return np.tanh(1.3 * (y + nz))


def noise_sweep(d, f0, f1, shape, rng=None, width=0.35):
    rng = rng or np.random.default_rng(2)
    x = rng.standard_normal(int(d * SR))
    y = stft_filter(x, lambda tc, fr: np.exp(-((np.log(fr + 1) - np.log(f0 * (f1 / f0) ** np.clip(tc / d, 0, 1))) ** 2) / width))
    k = np.arange(len(y)) / len(y)
    return y * shape(k)


def click(d=0.006, lo=1500, hi=9000, rng=None):
    rng = rng or np.random.default_rng(3)
    t = tt(d)
    x = rng.standard_normal(len(t)) * np.exp(-t / (d / 5))
    return fft_filter(x, lambda fr: hp_weight(fr, lo) * lp_weight(fr, hi))


def reverb(x, rt=2.4, pre=0.02, damp=5000, seed=5):
    rng = np.random.default_rng(seed)
    n_ir = int(rt * 1.3 * SR)
    t = np.arange(n_ir) / SR
    out = np.zeros_like(x)
    nfft = 1 << (x.shape[1] + n_ir + int(pre * SR) - 1).bit_length()
    for ch in range(2):
        ir = rng.standard_normal(n_ir) * np.exp(-6.9 * t / rt)
        ir = fft_filter(ir, lambda fr: lp_weight(fr, damp, 1) * hp_weight(fr, 150, 1))
        ir = np.concatenate([np.zeros(int(pre * SR)), ir])
        out[ch] = np.fft.irfft(np.fft.rfft(x[ch], nfft) * np.fft.rfft(ir, nfft), nfft)[: x.shape[1]]
    return out / (np.sqrt(np.mean(out ** 2)) + 1e-12) * np.sqrt(np.mean(x ** 2))


def ping_pong(x, delay, fb=0.4, taps=5, fc=3500):
    out = np.zeros_like(x)
    mono = x.sum(0) * 0.5
    d = int(delay * SR)
    for k in range(1, taps + 1):
        if k * d >= x.shape[1]:
            break
        s = np.zeros(x.shape[1])
        s[k * d:] = mono[: x.shape[1] - k * d] * fb ** k
        out[k % 2] += s
    return np.stack([fft_filter(out[c], lambda fr: lp_weight(fr, fc)) for c in range(2)])


def tape_stop(x, at, length, silence_until=None):
    """Slow the bus to a halt starting at `at`; silent afterwards until `silence_until`."""
    n = x.shape[1]
    i0, i1 = int(at * SR), int((at + length) * SR)
    pos = np.arange(n, dtype=float)
    k = np.arange(i1 - i0) / (i1 - i0)
    pos[i0:i1] = i0 + np.cumsum((1 - k) ** 1.6)
    out = np.stack([np.interp(pos, np.arange(n), x[c]) for c in range(2)])
    fade = np.ones(n)
    fade[i0:i1] = (1 - k) ** 0.7
    end = int((silence_until or at + length) * SR)
    fade[i1:end] = 0
    return out * fade


def db(x):
    return 20 * np.log10(np.sqrt(np.mean(x ** 2)) + 1e-12)


def write_wav(path, mix):
    mix = mix / (np.max(np.abs(mix)) / 0.89)
    mix = np.tanh(mix * 1.15) / np.tanh(1.15) * 0.89
    pcm = (np.clip(mix.T, -1, 1) * 32767).astype("<i2")
    with wave.open(path, "wb") as w:
        w.setnchannels(2)
        w.setsampwidth(2)
        w.setframerate(SR)
        w.writeframes(pcm.tobytes())


def bands(mix, t0, t1):
    seg = mix.mean(0)[int(t0 * SR):int(t1 * SR)]
    X = np.abs(np.fft.rfft(seg * np.hanning(len(seg)))) ** 2
    fr = np.fft.rfftfreq(len(seg), 1 / SR)
    tot = X.sum()
    return " ".join(f"{lo}-{hi}:{10 * np.log10(X[(fr >= lo) & (fr < hi)].sum() / tot):.1f}" for lo, hi in
                    [(20, 60), (60, 120), (120, 250), (250, 500), (500, 1000), (1000, 2000), (2000, 4000), (4000, 8000), (8000, 16000)])
