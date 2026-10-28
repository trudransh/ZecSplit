# Wire messages

The messages between taker and maker, and the maker's HTTP routes (spec §7).

Every message has the shape:

```json
{ "v": 1, "swap_id": "<hex32>", "type": "<Type>", "body": { } }
```

Bytes are lowercase hex. A `QuoteRequest` carries an all-zero `swap_id`. The signed form of a quote is sorted, compact JSON (JCS).

## Sequence

| #  | Type             | From   | Body                                                                         |
| -- | ---------------- | ------ | ---------------------------------------------------------------------------- |
| 1  | `QuoteRequest`   | A      | `{ zec_amount, network }`                                                    |
| 1  | `Quote`          | B      | quote fields + `maker_sig` (BIP340 over `H256(HK_quote_v1, canonical_json)`) |
| 2  | `Commit`         | A      | `{ commitment }`                                                             |
| 3  | `Commit`         | B      | `{ commitment }`                                                             |
| 4  | `Reveal`         | A      | shares, Bitcoin key, nonce, DLEq proof, proofs of knowledge, payout address  |
| 5  | `Reveal`         | B      | same                                                                         |
| 6  | `LockTemplate`   | B      | `{ psbt_base64, lock_vout, v_lock_sats, refund_addr }`                       |
| 7  | `SetupSigsAlice` | A      | `{ sig_cancel, esig_refund }`                                                |
| 8  | `SetupSigsBob`   | B      | `{ sig_cancel, sig_punish }`                                                 |
| 9  | `LockBroadcast`  | B      | `{ txid }` (informational; Alice verifies on chain)                          |
| 10 | `RedeemEncSig`   | B      | `{ esig_redeem }`                                                            |
| —  | `Status`         | either | `{ state, note }` (informational, never trusted)                             |
| —  | `Error`          | either | `{ code, message }`                                                          |

**§7-R1.** Every message carries the protocol version and `swap_id`. Unknown versions are rejected.

**§7-R2.** Transport is HTTPS to the maker. The taker MAY route it through Tor (SOCKS5).

## Maker HTTP routes

| Method | Route                         | Purpose                                                         |
| ------ | ----------------------------- | --------------------------------------------------------------- |
| `POST` | `/v1/quote`                   | Request a signed quote                                          |
| `POST` | `/v1/swaps/{id}/msg`          | Deliver one message; returns the maker's next message, or `204` |
| `GET`  | `/v1/swaps/{id}/next?wait=30` | Long-poll for the maker's next message                          |
| `GET`  | `/v1/info`                    | Maker identity key, limits and supported networks               |

The protocol crate knows nothing about HTTP. Transport lives in the binaries, so discovery and transport (relays, a quote board for many makers) can change without touching the state machines. Quotes are signed by the maker's identity key, so anything that relays them can't forge them.
