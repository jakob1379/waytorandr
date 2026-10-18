use std::collections::HashMap;
use std::time::Duration;

use zbus::blocking::connection::Builder;
use zbus::zvariant::Value;

const METHOD_TIMEOUT: Duration = Duration::from_secs(1);

/// Sends a desktop notification, ignoring delivery errors.
pub(crate) fn notify(summary: &str, body: &str, timeout_ms: i32) {
    let _ = try_notify(summary, body, timeout_ms);
}

/// Sends a transient notification over the session bus with the given expiry in milliseconds.
///
/// Requests replacement via the synchronous hint and limits the method reply wait
/// to one second. Returns an error if connecting to the bus or sending fails.
fn try_notify(summary: &str, body: &str, timeout_ms: i32) -> zbus::Result<()> {
    let connection = Builder::session()?.method_timeout(METHOD_TIMEOUT).build()?;
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
