# suspend-web

An axum-based Rust Linux service that listens on port `7878` and suspends the system when `GET /suspend` is called.

All endpoints return JSON.

## Endpoints

- `GET /` - health/info response
- `GET /suspend` - triggers a suspend request via `systemctl suspend` (fallback: `loginctl suspend`)
- `GET /games` - returns merged installed games from Steam and Steam shortcuts (Heroic/Epic) in the format `{ "id": "", "name": "", "source": "steam|epic" }` (excluding `Steamworks Common Redistributables`, `Steam Linux Runtime`, Proton entries, and obvious launcher shortcuts). Epic entries are discovered from Steam `shortcuts.vdf` and use Steam non-Steam `GameID` (`(shortcut_appid << 32) | 0x02000000`) as `id`.
- `POST /games/launch` - launches a game via Steam using a body like `{ "id": "4031200447", "source": "epic", "wait": true, "wait_timeout_ms": 15000 }` (`source`/`wait`/`wait_timeout_ms` are optional). `wait` defaults to `true` and the endpoint returns success only if a start signal is verified in time.

## Build

```bash
cd /home/witek/streaming
cargo build --release
```

Binary output:

- `target/release/suspend-web`

## Run manually

```bash
./target/release/suspend-web
```

## Install as systemd service

```bash
sudo cp target/release/suspend-web /usr/local/bin/suspend-web
sudo chmod +x /usr/local/bin/suspend-web
sudo cp /home/witek/streaming/suspend-web.service /etc/systemd/system/suspend-web.service
sudo systemctl daemon-reload
sudo systemctl enable --now suspend-web.service
sudo systemctl status suspend-web.service
```

## Test endpoints

```bash
curl http://127.0.0.1:7878/
curl http://127.0.0.1:7878/suspend
curl http://127.0.0.1:7878/games
curl -X POST http://127.0.0.1:7878/games/launch -H 'Content-Type: application/json' -d '{"id":"4031200447","source":"epic"}'
curl -X POST http://127.0.0.1:7878/games/launch -H 'Content-Type: application/json' -d '{"id":"4031200447","source":"epic","wait":true,"wait_timeout_ms":15000}'
```

## Optional launcher wrapper install

`POST /games/launch` can use a wrapper script for better Steam/Game Mode focus behavior.

```bash
sudo cp /home/witek/streaming/scripts/steam-launch.sh /usr/local/bin/suspend-web-launch
sudo chmod +x /usr/local/bin/suspend-web-launch
```

You can also point to a custom path with env var `SUSPEND_WEB_LAUNCHER`.

> Calling `/suspend` can put your machine to sleep immediately.
