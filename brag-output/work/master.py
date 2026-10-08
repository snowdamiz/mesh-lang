"""python3 master.py in.wav out.wav: gain to -14 LUFS, then a lookahead true-peak limiter at -1.5 dBTP."""
import re
import subprocess
import sys
import wave
import numpy as np
from numpy.lib.stride_tricks import sliding_window_view

SR = 48000


def lufs(path):
    out = subprocess.run(["ffmpeg", "-hide_banner", "-i", path, "-af", "loudnorm=print_format=json", "-f", "null", "-"],
                         capture_output=True, text=True).stderr
    return float(re.search(r'"input_i" : "(-?[\d.]+)"', out).group(1))


def read(path):
    with wave.open(path) as w:
        return np.frombuffer(w.readframes(w.getnframes()), "<i2").reshape(-1, 2).T / 32768.0


def write(path, x):
    with wave.open(path, "wb") as w:
        w.setnchannels(2)
        w.setsampwidth(2)
        w.setframerate(SR)
        w.writeframes((np.clip(x.T, -1, 1) * 32767).astype("<i2").tobytes())


def true_peak(x, over=4):
    n = x.shape[1]
    X = np.fft.rfft(x, axis=1)
    up = np.fft.irfft(X, n * over, axis=1) * over
    return np.abs(up).max(0).reshape(n, over).max(1)


def limit(x, ceiling_db=-1.5, win_s=0.004):
    ceil = 10 ** (ceiling_db / 20)
    need = np.minimum(1.0, ceil / np.maximum(true_peak(x), 1e-9))
    w = int(win_s * SR) | 1
    pad = np.pad(need, (w // 2, w // 2), mode="edge")
    gmin = sliding_window_view(pad, w).min(1)
    g = np.convolve(np.pad(gmin, (w // 2, w // 2), mode="edge"), np.ones(w) / w, mode="valid")
    # every smoothed value is an average of window minima that all include this sample, so g <= need
    return x * g


src, dst = sys.argv[1], sys.argv[2]
x = read(src)
for _ in range(3):  # limiting takes a little loudness back; converge on -14
    gain = -14.0 - lufs(src if _ == 0 else dst)
    x = limit(x * 10 ** (gain / 20))
    write(dst, x)
print(f"{lufs(dst):.1f} LUFS, sample-TP {20 * np.log10(true_peak(x).max()):.2f} dB")
