"""Render Castor's launch images and Open Graph images with headless Chrome.

    py video/social/src/render.py                 # all X/Twitter images -> video/social/*.png
    py video/social/src/render.py --og <dir>      # + og.png / og-fr.png (1200x630) into <dir>
    py video/social/src/render.py hero-1600x900   # only the named outputs

Needs Google Chrome. Serves the repository root on 127.0.0.1 (fonts come from assets/fonts,
the tool list from tools.json, made by video/site/catalogue.py).
"""
import argparse, functools, http.server, os, pathlib, shutil, subprocess, sys, tempfile, threading, time

SRC = pathlib.Path(__file__).resolve().parent
ROOT = SRC.parents[2]
OUT = SRC.parent
CHROME = os.environ.get("CHROME") or next((p for p in [
    r"C:\Program Files\Google\Chrome\Application\chrome.exe",
    r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"] if os.path.exists(p)), "google-chrome")

CARDS = ["hero", "catalogue", "hangar", "beyond", "download"]
JOBS = {}
for i, c in enumerate(CARDS, 1):
    for size in ["1600x900", "1080x1350"]:
        JOBS[f"{i}-{c}-{size}"] = dict(card=c, size=size, theme="light", lang="en")
for size in ["1600x900", "1080x1350"]:
    JOBS[f"1-hero-dark-{size}"] = dict(card="hero", size=size, theme="dark", lang="en")
# French set (fr/ subfolder): same cards, same sizes.
for i, c in enumerate(CARDS, 1):
    for size in ["1600x900", "1080x1350"]:
        JOBS[f"fr/{i}-{c}-{size}"] = dict(card=c, size=size, theme="light", lang="fr")
for size in ["1600x900", "1080x1350"]:
    JOBS[f"fr/1-hero-dark-{size}"] = dict(card="hero", size=size, theme="dark", lang="fr")
OG = {"og": dict(card="og", size="1200x630", theme="light", lang="en"),
      "og-fr": dict(card="og", size="1200x630", theme="light", lang="fr")}


def serve():
    class Quiet(http.server.SimpleHTTPRequestHandler):
        def log_message(self, *a):
            pass
    h = functools.partial(Quiet, directory=str(ROOT))
    httpd = http.server.ThreadingHTTPServer(("127.0.0.1", 0), h)
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    return httpd


def render(port, job, out: pathlib.Path):
    w, h = job["size"].split("x")
    q = "&".join(f"{k}={v}" for k, v in job.items())
    url = f"http://127.0.0.1:{port}/video/social/src/card.html?{q}"
    prof = tempfile.mkdtemp(prefix="castor-card-")
    if out.exists():
        out.unlink()
    subprocess.run([CHROME, "--headless=new", f"--user-data-dir={prof}", "--no-first-run", "--hide-scrollbars",
                    "--force-device-scale-factor=1", f"--window-size={w},{h}", "--virtual-time-budget=10000",
                    f"--screenshot={out}", url], capture_output=True, timeout=180)
    for _ in range(60):
        if out.exists() and out.stat().st_size:
            break
        time.sleep(0.5)
    shutil.rmtree(prof, ignore_errors=True)
    print(("ok  " if out.exists() else "FAIL"), out.name)


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("names", nargs="*")
    ap.add_argument("--og", type=pathlib.Path, help="folder for og.png and og-fr.png (the website's castor/img)")
    a = ap.parse_args()
    httpd = serve()
    port = httpd.server_address[1]
    jobs = {k: v for k, v in JOBS.items() if not a.names or k in a.names}
    for name, job in jobs.items():
        render(port, job, OUT / f"{name}.png")
    if a.og:
        for name, job in OG.items():
            render(port, job, a.og / f"{name}.png")
    httpd.shutdown()
