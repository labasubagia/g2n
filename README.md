# g2n

Upload Google Drive images to a Notion database, using each file's Drive description as the page prompt.

Scans a Drive folder for images, dedups against existing Notion page titles (querying only your filenames, chunked ≤100 per request), downloads and uploads the new ones, and writes a `Created Time` property when the source file has one.

## How it works

1. **List Drive images** — finds every image in the configured folder.
2. **Dedup** — for each batch of ≤100 filenames, queries the Notion database for title matches and skips those pages. This avoids scanning the whole database.
3. **Download + upload** — new images are downloaded and uploaded to Notion with the Drive description as the prompt, rate-limited so uploads stay under Notion's per-second budget.

## Setup

1. Create the Notion database with these properties:
   - `Name` — Title
   - `Image` — Files
   - `Created Time` — Date (optional)
2. Create a Notion integration and share the database with it.
3. Create a Google Cloud OAuth desktop client.
4. Copy `config.toml.example` → `config.toml` and fill in:
   - `[google]` — client id/secret, Drive folder id
   - `[notion]` — integration token, database id, `requests_per_second`

`config.toml` is gitignored and never committed.

## Usage

```sh
cargo run --release -- sync
```

Options:

- `--config <path>` — path to `config.toml` (defaults to platform config dir)
- `--download-dir <dir>` — where images are downloaded (defaults to platform data dir `/images`)
- `--no-interactive` — fail instead of opening a browser when no token is cached
- `--concurrency <n>` — max concurrent downloads/uploads (default 4)

First run opens a browser for Google authorization; the token is cached.

## Development

```sh
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Tests use wiremock + tempfile — no live Drive/Notion calls, no credentials.
