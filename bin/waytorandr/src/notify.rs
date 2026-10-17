use std::collections::HashMap;

use zbus::blocking::Connection;
use zbus::zvariant::Value;

const EXPIRE_TIMEOUT_MS: i32 = 3000;

// Best-effort: a missing session bus or notification server must not fail the command.
pub(crate) fn notify(summary: &str, body: &str) {
    let _ = try_notify(summary, body);
}

fn try_notify(summary: &str, body: &str) -> zbus::Result<()> {
    let connection = Connection::session()?;
    // Lets servers that support it replace the previous popup instead of stacking one per cycle.
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
            EXPIRE_TIMEOUT_MS,
        ),
    )?;
    Ok(())
}
