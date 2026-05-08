use futures_util::StreamExt;
use gilrs::{Button, EventType, Gilrs};
use tokio::sync::mpsc;
use zbus::proxy;

#[proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait Login1Manager {
    #[zbus(signal)]
    fn prepare_for_sleep(&self, start: bool) -> zbus::Result<()>;
}

// ── Event hooks ───────────────────────────────────────────────────────────────

/// Called once when the service process starts (boot or manual start).
pub async fn on_boot() {
    println!("[events] boot");
}

/// Called when the system is about to suspend/hibernate.
pub async fn on_sleep() {
    println!("[events] going to sleep");
}

/// Called after the system wakes up from suspend/hibernate.
pub async fn on_wake() {
    println!("[events] woke up from sleep");
}

/// Called when a gamepad is connected.
pub async fn on_gamepad_connected(name: &str) {
    println!("[events] controller connected: {name}");
}

/// Called when a gamepad is disconnected.
pub async fn on_gamepad_disconnected(name: &str) {
    println!("[events] controller disconnected: {name}");
}

/// Called when any button is pressed on any connected gamepad.
pub async fn on_button_pressed(name: &str, button: Button) {
    println!("[events] button {button:?} pressed on {name}");
}

// ── Sleep listener ────────────────────────────────────────────────────────────

/// Spawns a background task that listens for system sleep/wake events over
/// D-Bus (`org.freedesktop.login1` `PrepareForSleep` signal) and calls the
/// [`on_sleep`] / [`on_wake`] hooks accordingly.
///
/// The task reconnects automatically if the D-Bus connection is lost.
pub fn spawn_sleep_listener() {
    tokio::spawn(async move {
        loop {
            match run_sleep_listener().await {
                Ok(()) => {
                    eprintln!("[events] D-Bus stream ended unexpectedly, reconnecting...");
                }
                Err(error) => {
                    eprintln!("[events] D-Bus error: {error}, reconnecting in 5s...");
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                }
            }
        }
    });
}

async fn run_sleep_listener() -> zbus::Result<()> {
    let conn = zbus::Connection::system().await?;
    let proxy = Login1ManagerProxy::new(&conn).await?;
    let mut stream = proxy.receive_prepare_for_sleep().await?;

    println!("[events] listening for sleep/wake events via D-Bus");

    while let Some(signal) = stream.next().await {
        let args = signal.args()?;
        if args.start {
            on_sleep().await;
        } else {
            on_wake().await;
        }
    }

    Ok(())
}

// ── Gamepad listener ──────────────────────────────────────────────────────────

enum GamepadEvent {
    Connected { name: String },
    Disconnected { name: String },
    ButtonPressed { name: String, button: Button },
}

/// Spawns a background thread running the gilrs event loop and a tokio task
/// that consumes the events and calls the gamepad hooks.
pub fn spawn_gilrs_listener() {
    let (tx, mut rx) = mpsc::channel::<GamepadEvent>(64);

    std::thread::spawn(move || {
        let mut gilrs = match Gilrs::new() {
            Ok(g) => g,
            Err(error) => {
                eprintln!("[events] failed to init gilrs: {error}");
                return;
            }
        };

        println!("[events] gilrs ready, listening for gamepad events");

        loop {
            while let Some(gilrs::Event { id, event, .. }) = gilrs.next_event() {
                let name = gilrs.gamepad(id).name().to_string();
                let evt = match event {
                    EventType::Connected => GamepadEvent::Connected { name },
                    EventType::Disconnected => GamepadEvent::Disconnected { name },
                    EventType::ButtonPressed(button, _) => {
                        GamepadEvent::ButtonPressed { name, button }
                    }
                    _ => continue,
                };
                if tx.blocking_send(evt).is_err() {
                    return; // tokio side dropped, shut down
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(8));
        }
    });

    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            match event {
                GamepadEvent::Connected { name } => on_gamepad_connected(&name).await,
                GamepadEvent::Disconnected { name } => on_gamepad_disconnected(&name).await,
                GamepadEvent::ButtonPressed { name, button } => {
                    on_button_pressed(&name, button).await;
                }
            }
        }
    });
}
