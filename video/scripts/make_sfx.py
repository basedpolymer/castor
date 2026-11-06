"""Subtle UI sounds for the promo video, synthesised from scratch (no samples, no licence
question): clicks, key taps, a wooden tick, a paper drop, a two-note chime, a pencil stroke and
a beaver's gnaw. Python standard library only.

    py -I video/scripts/make_sfx.py video/public/sfx
"""
import math
import os
import random
import struct
import sys
import wave

SR = 48000
random.seed(7)


def write(path, samples, gain=1.0):
    peak = max(1e-9, max(abs(s) for s in samples))
    k = min(0.9, gain * 1.8) / peak  # +5 dB over the first mix, still subtle
    with wave.open(path, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(SR)
        w.writeframes(b"".join(struct.pack("<h", int(max(-1, min(1, s * k)) * 32767)) for s in samples))


def n(sec):
    return int(sec * SR)


def noise(count):
    return [random.uniform(-1, 1) for _ in range(count)]


def lowpass(x, fc):
    a = 1 - math.exp(-2 * math.pi * fc / SR)
    y, s = [], 0.0
    for v in x:
        s += a * (v - s)
        y.append(s)
    return y


def highpass(x, fc):
    low = lowpass(x, fc)
    return [a - b for a, b in zip(x, low)]


def bandpass(x, lo, hi):
    return lowpass(highpass(x, lo), hi)


def env(count, attack, decay):
    a = max(1, n(attack))
    return [(i / a if i < a else 1.0) * math.exp(-(i - a) / max(1, n(decay))) if i >= a else i / a for i in range(count)]


def mix(*tracks):
    m = max(len(t) for t in tracks)
    return [sum(t[i] for t in tracks if i < len(t)) for i in range(m)]


def tone(freq, sec, attack, decay, amp=1.0):
    c = n(sec)
    e = env(c, attack, decay)
    return [amp * e[i] * math.sin(2 * math.pi * freq * i / SR) for i in range(c)]


def pad(x, sec):
    return [0.0] * n(sec) + x


def click():
    c = n(0.03)
    e = env(c, 0.0005, 0.004)
    body = [e[i] * v for i, v in enumerate(bandpass(noise(c), 1800, 7000))]
    return mix(body, tone(2600, 0.03, 0.0003, 0.006, 0.5))


def key(seed):
    random.seed(seed)
    c = n(0.035)
    e = env(c, 0.0008, 0.006 + 0.002 * (seed % 3))
    body = [e[i] * v for i, v in enumerate(bandpass(noise(c), 900 + 150 * (seed % 4), 5000))]
    return mix(body, tone(1500 + 120 * (seed % 5), 0.03, 0.0005, 0.005, 0.25))


def tick():
    return mix(tone(1650, 0.08, 0.0005, 0.018), tone(3300, 0.06, 0.0005, 0.008, 0.35))


def drop():
    c = n(0.22)
    e = env(c, 0.002, 0.05)
    thud = [e[i] * v for i, v in enumerate(lowpass(noise(c), 420))]
    return mix(thud, tone(120, 0.22, 0.003, 0.06, 0.6), [0.3 * v for v in bandpass(noise(n(0.05)), 2000, 6000)])


def chime():
    a = mix(tone(784, 1.2, 0.006, 0.35, 0.8), tone(1568, 1.0, 0.004, 0.18, 0.15))
    b = mix(tone(1175, 1.2, 0.006, 0.45, 0.7), tone(2350, 1.0, 0.004, 0.2, 0.12))
    return mix(a, pad(b, 0.11))


def pencil():
    c = n(1.6)
    x = bandpass(noise(c), 2500, 6500)
    out = []
    for i, v in enumerate(x):
        t = i / SR
        strokes = 0.55 + 0.45 * math.sin(2 * math.pi * 3.1 * t) ** 2
        fade = min(1.0, t / 0.08) * min(1.0, (1.6 - t) / 0.25)
        out.append(v * strokes * fade)
    return out


def gnaw():
    out = []
    for k in range(3):
        c = n(0.05)
        e = env(c, 0.001, 0.012)
        burst = [e[i] * v for i, v in enumerate(bandpass(noise(c), 1500 + 400 * k, 6000))]
        out = mix(out, pad(burst, 0.085 * k))
    return out


def crumb():
    return mix(tone(3400, 0.05, 0.0003, 0.007, 0.6), tone(5100, 0.04, 0.0003, 0.004, 0.25),
               [0.25 * v for v in bandpass(noise(n(0.02)), 3000, 9000)])


def main():
    out = sys.argv[1]
    os.makedirs(out, exist_ok=True)
    write(os.path.join(out, "click.wav"), click(), 0.5)
    for s in range(4):
        write(os.path.join(out, f"key{s}.wav"), key(s + 1), 0.3)
    write(os.path.join(out, "tick.wav"), tick(), 0.28)
    write(os.path.join(out, "drop.wav"), drop(), 0.5)
    write(os.path.join(out, "chime.wav"), chime(), 0.35)
    write(os.path.join(out, "pencil.wav"), pencil(), 0.12)
    write(os.path.join(out, "gnaw.wav"), gnaw(), 0.4)
    write(os.path.join(out, "crumb.wav"), crumb(), 0.35)


main()
