use axum::{
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use tokio::net::TcpListener;

mod events;
mod handlers;
mod launcher;
mod library;
mod poster;
mod process;
mod suspend;

use handlers::games::{
    game_kill_by_id_handler, games_handler, games_kill_handler, games_launch_handler,
    games_running_handler, games_running_stop_handler,
};
use handlers::poster::get_game_poster_handler;
use handlers::suspend::suspend_handler;

const BIND_ADDR: &str = "0.0.0.0:7878";

#[tokio::main]
async fn main() -> std::io::Result<()> {
    events::on_boot().await;
    events::spawn_sleep_listener();
    events::spawn_gilrs_listener();

    let listener = TcpListener::bind(BIND_ADDR).await?;
    println!("suspend-web listening on http://{BIND_ADDR}");

    axum::serve(listener, app()).await
}

fn app() -> Router {
    let games_routes = Router::new()
        .route("/", get(games_handler))
        .route("/launch", post(games_launch_handler))
        .route("/kill", post(games_kill_handler))
        .route("/running", get(games_running_handler))
        .route("/running/stop", post(games_running_stop_handler))
        .route("/:id/kill", post(game_kill_by_id_handler))
        .route("/:id/poster", get(get_game_poster_handler));

    Router::new()
        .route("/", get(root_handler))
        .route("/suspend", get(suspend_handler))
        .nest("/games", games_routes)
}

async fn root_handler() -> Json<Value> {
    Json(json!({
        "service": "suspend-web",
        "status": "ok",
        "message": "Service is running",
        "endpoints": ["/", "/suspend", "/games", "/games/launch", "/games/kill", "/games/running", "/games/running/stop", "/games/:id/kill", "/games/:id/poster"]
    }))
}
