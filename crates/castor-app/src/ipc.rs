//! Single instance: the first Castor listens on a local socket; later launches forward their
//! arguments (files, `--tool`) to it and exit.

use std::io::{BufRead, BufReader, Write};

use crossbeam_channel::{Receiver, Sender, unbounded};
use interprocess::local_socket::{GenericNamespaced, ListenerOptions, Stream, ToNsName, prelude::*};

use crate::cli::Args;

fn socket_name() -> String {
    // One instance per user and per profile (CASTOR_HOME makes a separate instance).
    let user = std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_default();
    let home = std::env::var("CASTOR_HOME").unwrap_or_default();
    let mut h: u64 = 1469598103934665603;
    for b in format!("{user}|{home}").bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    format!("eu.basedpolymer.castor-{h:016x}.sock")
}

/// Try to hand `args` to a running instance. Returns true if one took them.
pub fn forward_to_running(args: &Args) -> bool {
    let Ok(name) = socket_name().to_ns_name::<GenericNamespaced>() else { return false };
    let Ok(mut conn) = Stream::connect(name) else { return false };
    let Ok(line) = serde_json::to_string(args) else { return false };
    if conn.write_all(line.as_bytes()).and_then(|_| conn.write_all(b"\n")).and_then(|_| conn.flush()).is_err() {
        return false;
    }
    // Wait for the ack so the other side has read everything before we exit.
    let mut ack = String::new();
    let _ = BufReader::new(conn).read_line(&mut ack);
    true
}

/// Start listening. Messages arrive on the returned channel (polled by the UI).
pub fn listen() -> Option<Receiver<Args>> {
    let name = socket_name().to_ns_name::<GenericNamespaced>().ok()?;
    let listener = match ListenerOptions::new().name(name).create_sync() {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("single-instance socket unavailable: {e}");
            return None;
        }
    };
    let (tx, rx): (Sender<Args>, Receiver<Args>) = unbounded();
    std::thread::Builder::new()
        .name("castor-ipc".into())
        .spawn(move || {
            for conn in listener.incoming() {
                let Ok(conn) = conn else { continue };
                let mut reader = BufReader::new(conn);
                let mut line = String::new();
                if reader.read_line(&mut line).is_ok() {
                    if let Ok(args) = serde_json::from_str::<Args>(line.trim()) {
                        let _ = tx.send(args);
                    }
                    let _ = reader.get_mut().write_all(b"ok\n");
                }
            }
        })
        .ok()?;
    Some(rx)
}
