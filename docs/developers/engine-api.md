# Engine API

The localhost API between the web UI and the zecsplitd engine.

The engine API is the only contract the UI sees. The web UI uses it today; a desktop or mobile shell can use the same API later. Its types are Rust structs exported to TypeScript with `ts-rs`, so the frontend's types can't drift from the engine's.

> [!NOTE]
> The HTTP server is in progress. Routes below are the contract it implements.

| Method | Route                            | Purpose                                                                                              |
| ------ | -------------------------------- | ---------------------------------------------------------------------------------------------------- |
| `GET`  | `/api/health`                    | Engine version, network, whether it is unlocked                                                      |
| `POST` | `/api/unlock`                    | `{ passphrase }`. Unlocks the sealed sessions for this run                                           |
| `POST` | `/api/quote`                     | `{ zec_amount }` → the signed quote, rendered                                                        |
| `POST` | `/api/swaps`                     | `{ quote_id, btc_address, accept_risks: true }` → `{ swap_id }`                                      |
| `GET`  | `/api/swaps/{id}/recovery-file`  | Download the encrypted recovery file                                                                 |
| `POST` | `/api/swaps/{id}/recovery-saved` | The user confirms they saved it                                                                      |
| `GET`  | `/api/swaps`                     | Swaps with their state                                                                               |
| `GET`  | `/api/swaps/{id}`                | Full view: state, steps, txids, confirmations, deadlines, and the ZIP 321 URI when it is time to pay |
| `GET`  | `/api/swaps/{id}/events`         | Server-sent events stream of view updates                                                            |
| `POST` | `/api/restore`                   | Upload a recovery file and passphrase                                                                |

## Security

* Binds to `127.0.0.1` only.
* Requires an `Origin` allowlist (the local UI origin).
* Uses a random per-session token injected into the served UI.
* Never returns secret material.

## The swap runner behind it

The engine drives each swap with a runner that talks to the outside world through small interfaces:

| Port                      | Does                                                                              |
| ------------------------- | --------------------------------------------------------------------------------- |
| `Maker`                   | Sends a message to the maker and polls for its next one                           |
| `BtcSource` (× 2 or more) | Reads the Bitcoin tip and transactions; a fact counts only if every source agrees |
| `Broadcast`               | Broadcasts a Bitcoin transaction, idempotently                                    |
| `ZecSweep`                | Sweeps the lock back on the refund path                                           |

On every tick it reads pending maker messages, takes a fresh snapshot of both chains, steps the state machine, saves, then acts. On start it resumes every unfinished swap from SQLite.
