use std::collections::HashMap;
use std::time::Duration;

use zbus::blocking::connection::Builder;
use zbus::zvariant::Value;

const EXPIRE_TIMEOUT_MS: i32 = 3000;
const METHOD_TIMEOUT: Duration = Duration::from_secs(1);

pub(crate) fn notify(summary: &str, body: &str) {
    let _ = try_notify(summary, body);
}

fn try_notify(summary: &str, body: &str) -> zbus::Result<()> {
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
            EXPIRE_TIMEOUT_MS,
        ),
    )?;
    Ok(())
}
