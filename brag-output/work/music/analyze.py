"""Tempo, beat phase and a coarse energy/brightness map for each track (numpy only)."""
import subprocess, sys, numpy as np
SR = 22050
def load(path):
    raw = subprocess.run(["ffmpeg", "-v", "error", "-i", path, "-ac", "1", "-ar", str(SR), "-f", "f32le", "-"], capture_output=True).stdout
    return np.frombuffer(raw, np.float32)
def analyze(path, show=True):
    x = load(path); dur = len(x) / SR
    hop, n = 512, 2048
    frames = np.lib.stride_tricks.sliding_window_view(x, n)[::hop] * np.hanning(n)
    S = np.abs(np.fft.rfft(frames, axis=1))
    fr = np.fft.rfftfreq(n, 1 / SR)
    flux = np.maximum(0, np.diff(np.log1p(S), axis=0)).sum(1)
    flux = flux - np.convolve(flux, np.ones(16) / 16, "same")
    flux = np.maximum(flux, 0)
    fps = SR / hop
    ac = np.correlate(flux, flux, "full")[len(flux) - 1:]
    lags = np.arange(len(ac)) / fps
    ok = (lags > 60 / 180) & (lags < 60 / 70)
    bpm_lag = lags[ok][np.argmax(ac[ok])]
    bpm = 60 / bpm_lag
    # beat phase: comb over the onset envelope
    period = bpm_lag * fps
    best = max(range(int(period)), key=lambda ph: flux[np.arange(ph, len(flux) - 1, period).astype(int)].sum())
    first_beat = best / fps
    # 2 s map
    seg = int(2 * SR)
    rms = [20 * np.log10(np.sqrt(np.mean(x[i:i + seg] ** 2)) + 1e-9) for i in range(0, len(x) - seg + 1, seg)]
    cen = []
    for i in range(0, len(x) - seg + 1, seg):
        s = np.abs(np.fft.rfft(x[i:i + seg])); f = np.fft.rfftfreq(seg, 1 / SR); cen.append((s * f).sum() / (s.sum() + 1e-9))
    low = []
    for i in range(0, len(x) - seg + 1, seg):
        s = np.abs(np.fft.rfft(x[i:i + seg])) ** 2; f = np.fft.rfftfreq(seg, 1 / SR); low.append(10 * np.log10(s[f < 120].sum() / (s.sum() + 1e-12) + 1e-12))
    if show:
        print(f"== {path}  {dur:.1f}s  ~{bpm:.1f} BPM  first beat {first_beat:.2f}s")
        bars = " ▁▂▃▄▅▆▇█"
        lo, hi = min(rms), max(rms)
        print("  loud  " + "".join(bars[int((r - lo) / (hi - lo + 1e-9) * 8)] for r in rms))
        clo, chi = min(cen), max(cen)
        print("  brite " + "".join(bars[int((c - clo) / (chi - clo + 1e-9) * 8)] for c in cen))
        print("  bass  " + "".join(bars[int(np.clip((l + 20) / 20, 0, 1) * 8)] for l in low))
        print("  (each char = 2 s)  rms dB range", round(lo, 1), "to", round(hi, 1))
    return dict(dur=dur, bpm=bpm, first=first_beat, rms=rms)
if __name__ == "__main__":
    for p in sys.argv[1:]:
        analyze(p)
