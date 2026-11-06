//! A minimal Chrome DevTools Protocol client over a local WebSocket (tungstenite, no TLS):
//! launches an installed Chromium-based browser headless with a throw-away profile, loads a
//! page, takes screenshots and prints PDFs. Nothing is downloaded; if no browser is installed
//! the HTML tools say so.

use std::io::ErrorKind;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use castor_core::RunCx;
use serde_json::{Value, json};
use tungstenite::{Message, WebSocket};

/// Find Chrome, Edge, Brave or Chromium. `CASTOR_CHROME` overrides.
pub fn find_browser() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("CASTOR_CHROME").map(PathBuf::from) {
        if p.is_file() {
            return Some(p);
        }
    }
    let mut c: Vec<PathBuf> = Vec::new();
    #[cfg(windows)]
    {
        let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
        for base in [env("ProgramFiles"), env("ProgramFiles(x86)"), env("LOCALAPPDATA")].into_iter().flatten() {
            c.push(base.join(r"Google\Chrome\Application\chrome.exe"));
            c.push(base.join(r"Microsoft\Edge\Application\msedge.exe"));
            c.push(base.join(r"BraveSoftware\Brave-Browser\Application\brave.exe"));
            c.push(base.join(r"Chromium\Application\chrome.exe"));
            c.push(base.join(r"Vivaldi\Application\vivaldi.exe"));
        }
    }
    #[cfg(target_os = "macos")]
    {
        for app in [
            "Google Chrome.app/Contents/MacOS/Google Chrome",
            "Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            "Brave Browser.app/Contents/MacOS/Brave Browser",
            "Chromium.app/Contents/MacOS/Chromium",
            "Vivaldi.app/Contents/MacOS/Vivaldi",
        ] {
            c.push(Path::new("/Applications").join(app));
            if let Some(h) = std::env::var_os("HOME") {
                c.push(PathBuf::from(h).join("Applications").join(app));
            }
        }
    }
    if let Some(p) = c.into_iter().find(|p| p.is_file()) {
        return Some(p);
    }
    for n in ["chrome", "google-chrome", "chromium", "chromium-browser", "msedge", "microsoft-edge", "brave-browser"] {
        if let Some(p) = castor_core::paths::find_program(n) {
            return Some(p);
        }
    }
    None
}

pub struct Browser {
    child: Child,
    ws: WebSocket<TcpStream>,
    profile: tempfile::TempDir,
    next_id: u64,
    session: Option<String>,
    cx: Option<RunCx>,
    /// Events received while waiting for a response.
    events: Vec<Value>,
}

impl Browser {
    pub fn launch(cx: Option<&RunCx>) -> Result<Browser> {
        let exe = find_browser().ok_or_else(|| anyhow!("{}", castor_core::tr("create.html.err.no_browser")))?;
        let profile = tempfile::Builder::new().prefix("castor-chrome-").tempdir()?;
        let mut cmd = castor_core::process::command(&exe);
        cmd.args([
            "--headless=new",
            "--disable-gpu",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-extensions",
            "--disable-sync",
            "--disable-background-networking",
            "--disable-component-update",
            "--disable-default-apps",
            "--metrics-recording-only",
            "--hide-scrollbars",
            "--mute-audio",
            "--font-render-hinting=none",
            "--remote-debugging-port=0",
            "--remote-allow-origins=*",
        ]);
        cmd.arg(format!("--user-data-dir={}", profile.path().display()));
        cmd.arg("about:blank");
        cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        let mut child = cmd.spawn().map_err(|e| anyhow!("could not start {}: {e}", exe.display()))?;
        // Chrome writes the port it picked into DevToolsActivePort.
        let port_file = profile.path().join("DevToolsActivePort");
        let t0 = Instant::now();
        let (port, path) = loop {
            if let Ok(s) = std::fs::read_to_string(&port_file) {
                let mut l = s.lines();
                if let (Some(p), Some(path)) = (l.next(), l.next()) {
                    if let Ok(p) = p.trim().parse::<u16>() {
                        break (p, path.trim().to_string());
                    }
                }
            }
            // Chrome on Windows may hand over to a relaunched process and exit with 0: keep
            // waiting for the port file; only a failure status is fatal.
            if let Some(st) = child.try_wait()? {
                if !st.success() {
                    bail!("the browser exited at start ({st})");
                }
            }
            if let Some(cx) = cx {
                if cx.is_cancelled() {
                    let _ = child.kill();
                    return Err(castor_core::job::Cancelled.into());
                }
            }
            if t0.elapsed() > Duration::from_secs(30) {
                let _ = child.kill();
                bail!("the browser did not start within 30 s");
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        let stream = TcpStream::connect(("127.0.0.1", port))?;
        stream.set_read_timeout(Some(Duration::from_secs(20)))?;
        let cfg = tungstenite::protocol::WebSocketConfig::default()
            .max_message_size(Some(1 << 30))
            .max_frame_size(Some(1 << 30));
        let (ws, _) = tungstenite::client::client_with_config(format!("ws://127.0.0.1:{port}{path}"), stream, Some(cfg))
            .map_err(|e| anyhow!("DevTools handshake failed: {e}"))?;
        ws.get_ref().set_read_timeout(Some(Duration::from_millis(200)))?;
        Ok(Browser { child, ws, profile, next_id: 1, session: None, cx: cx.cloned(), events: Vec::new() })
    }

    fn check(&self) -> Result<()> {
        if let Some(cx) = &self.cx {
            cx.check_cancel()?;
        }
        Ok(())
    }

    fn read(&mut self) -> Result<Option<Value>> {
        match self.ws.read() {
            Ok(Message::Text(t)) => Ok(Some(serde_json::from_str(t.as_str())?)),
            Ok(_) => Ok(None),
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => Ok(None),
            Err(e) => Err(anyhow!("DevTools connection lost: {e}")),
        }
    }

    /// Send a command (to the attached page when `page`) and wait for its result.
    pub fn call(&mut self, method: &str, params: Value, page: bool, timeout: Duration) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let mut msg = json!({"id": id, "method": method, "params": params});
        if page {
            if let Some(s) = &self.session {
                msg["sessionId"] = json!(s);
            }
        }
        self.ws.send(Message::text(msg.to_string())).map_err(|e| anyhow!("DevTools send failed: {e}"))?;
        let t0 = Instant::now();
        loop {
            self.check()?;
            if t0.elapsed() > timeout {
                bail!("{method}: no answer from the browser after {} s", timeout.as_secs());
            }
            let Some(v) = self.read()? else { continue };
            if v.get("id").and_then(Value::as_u64) == Some(id) {
                if let Some(e) = v.get("error") {
                    bail!("{method}: {}", e.get("message").and_then(Value::as_str).unwrap_or("error"));
                }
                return Ok(v.get("result").cloned().unwrap_or(Value::Null));
            }
            if v.get("method").is_some() {
                self.events.push(v);
            }
        }
    }

    /// Wait for a page event (e.g. "Page.loadEventFired").
    pub fn wait_event(&mut self, method: &str, timeout: Duration) -> Result<bool> {
        if let Some(i) = self.events.iter().position(|e| e.get("method").and_then(Value::as_str) == Some(method)) {
            self.events.remove(i);
            return Ok(true);
        }
        let t0 = Instant::now();
        while t0.elapsed() < timeout {
            self.check()?;
            if let Some(v) = self.read()? {
                if v.get("method").and_then(Value::as_str) == Some(method) {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// Open a fresh tab and attach to it.
    pub fn new_page(&mut self) -> Result<()> {
        let t = self.call("Target.createTarget", json!({"url": "about:blank"}), false, Duration::from_secs(20))?;
        let target = t["targetId"].as_str().ok_or_else(|| anyhow!("no target id"))?.to_string();
        let a = self.call("Target.attachToTarget", json!({"targetId": target, "flatten": true}), false, Duration::from_secs(20))?;
        self.session = Some(a["sessionId"].as_str().ok_or_else(|| anyhow!("no session id"))?.to_string());
        self.call("Page.enable", json!({}), true, Duration::from_secs(20))?;
        self.call("Runtime.enable", json!({}), true, Duration::from_secs(20))?;
        Ok(())
    }

    pub fn viewport(&mut self, w: u32, h: u32, scale: f64, dark: bool, transparent: bool) -> Result<()> {
        self.call(
            "Emulation.setDeviceMetricsOverride",
            json!({"width": w, "height": h, "deviceScaleFactor": scale, "mobile": false}),
            true,
            Duration::from_secs(20),
        )?;
        self.call(
            "Emulation.setEmulatedMedia",
            json!({"features": [{"name": "prefers-color-scheme", "value": if dark { "dark" } else { "light" }}]}),
            true,
            Duration::from_secs(20),
        )?;
        if transparent {
            self.call("Emulation.setDefaultBackgroundColorOverride", json!({"color": {"r": 0, "g": 0, "b": 0, "a": 0}}), true, Duration::from_secs(20))?;
        }
        Ok(())
    }

    /// Navigate and wait for the load event, web fonts, then `delay`.
    pub fn goto(&mut self, url: &str, delay: Duration) -> Result<()> {
        self.events.clear();
        let r = self.call("Page.navigate", json!({"url": url}), true, Duration::from_secs(60))?;
        if let Some(e) = r.get("errorText").and_then(Value::as_str) {
            bail!("{}", castor_core::tr_args("create.html.err.load", &[("url", url), ("error", e)]));
        }
        if !self.wait_event("Page.loadEventFired", Duration::from_secs(60))? {
            bail!("{}", castor_core::tr_args("create.html.err.load", &[("url", url), ("error", "timeout")]));
        }
        let _ = self.call(
            "Runtime.evaluate",
            json!({"expression": "document.fonts ? document.fonts.ready.then(() => true) : true", "awaitPromise": true}),
            true,
            Duration::from_secs(20),
        );
        let t0 = Instant::now();
        while t0.elapsed() < delay {
            self.check()?;
            std::thread::sleep(Duration::from_millis(50));
        }
        Ok(())
    }

    pub fn eval_f64(&mut self, expr: &str) -> Result<f64> {
        let r = self.call("Runtime.evaluate", json!({"expression": expr, "returnByValue": true}), true, Duration::from_secs(20))?;
        r["result"]["value"].as_f64().ok_or_else(|| anyhow!("could not evaluate {expr}"))
    }

    /// PNG screenshot of the viewport, or of the whole page.
    pub fn screenshot(&mut self, w: u32, h: u32, scale: f64, full: bool) -> Result<Vec<u8>> {
        let mut height = h;
        if full {
            let sh = self.eval_f64("Math.max(document.documentElement.scrollHeight, document.body ? document.body.scrollHeight : 0)")?;
            // Chromium caps textures at 16 384 px.
            height = (sh.ceil() as u32).clamp(1, (16_000.0 / scale) as u32);
            self.call(
                "Emulation.setDeviceMetricsOverride",
                json!({"width": w, "height": height, "deviceScaleFactor": scale, "mobile": false}),
                true,
                Duration::from_secs(20),
            )?;
            std::thread::sleep(Duration::from_millis(150));
        }
        let r = self.call(
            "Page.captureScreenshot",
            json!({"format": "png", "captureBeyondViewport": full, "fromSurface": true,
                   "clip": {"x": 0, "y": 0, "width": w, "height": height, "scale": 1}}),
            true,
            Duration::from_secs(120),
        )?;
        crate::common::base64_decode(r["data"].as_str().unwrap_or(""))
    }

    pub fn pdf(&mut self, params: Value) -> Result<Vec<u8>> {
        let r = self.call("Page.printToPDF", params, true, Duration::from_secs(180))?;
        crate::common::base64_decode(r["data"].as_str().unwrap_or(""))
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        let _ = self.ws.send(Message::text(json!({"id": 0, "method": "Browser.close"}).to_string()));
        // The browser closes the connection when it has quit (the launched process may already
        // be gone if Chrome relaunched itself, so the socket is what tells).
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_secs(5) {
            match self.ws.read() {
                Err(tungstenite::Error::Io(e)) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
                Err(_) => break,
                Ok(_) => {}
            }
        }
        let t1 = Instant::now();
        while t1.elapsed() < Duration::from_secs(2) {
            if let Ok(Some(_)) = self.child.try_wait() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        // The profile folder may stay locked for a moment on Windows.
        for _ in 0..10 {
            if std::fs::remove_dir_all(self.profile.path()).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

/// `file:///…` URL for a local path.
pub fn file_url(p: &Path) -> String {
    let abs = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let s = abs.to_string_lossy().replace('\\', "/");
    let s = s.trim_start_matches("//?/").to_string();
    let mut o = String::from("file:///");
    for b in s.trim_start_matches('/').bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~:".contains(&b) {
            o.push(b as char);
        } else {
            o.push_str(&format!("%{b:02X}"));
        }
    }
    o
}
