use std::collections::HashMap;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use zbus::blocking::Connection;
use zbus::zvariant::Value;

const NOTIFY_TIMEOUT: Duration = Duration::from_secs(1);

/// Sends a desktop notification, ignoring delivery errors.
///
/// Delivery runs on a detached worker so a stalled session bus cannot block the
/// caller for longer than one second; the worker dies with the process.
pub(crate) fn notify(summary: &str, body: &str, timeout_ms: i32) {
    let (summary, body) = (summary.to_owned(), body.to_owned());
    let (done, finished) = mpsc::channel();
    let spawned = thread::Builder::new().spawn(move || {
        let _ = try_notify(&summary, &body, timeout_ms);
        let _ = done.send(());
    });
    if spawned.is_ok() {
        let _ = finished.recv_timeout(NOTIFY_TIMEOUT);
    }
}

/// Sends a transient notification over the session bus with the given expiry in milliseconds.
///
/// Requests replacement via the synchronous hint. Returns an error if connecting
/// to the bus or sending fails.
fn try_notify(summary: &str, body: &str, timeout_ms: i32) -> zbus::Result<()> {
    let connection = Connection::session()?;
    let hints = HashMap::from([
        ("transient", Value::from(true)),
        ("x-canonical-private-synchronous", Value::from("waytorandr")),
    ]);
    connection.call_method(
        Some("org.freedesktop.Notifications"),
        "/org/freedesktop/Notifications",
        Some("org.freedesktop.Notifications"),
        "Notify",
        &(
            "waytorandr",
            0_u32,
            "video-display",
            summary,
            body,
            Vec::<&str>::new(),
            hints,
            timeout_ms,
        ),
    )?;
    Ok(())
}
