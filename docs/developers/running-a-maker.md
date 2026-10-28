# Running a maker

What a ZecSplit maker is and what it needs.

The maker (Bob) is a standalone service that anyone can run. It quotes ZEC/BTC, locks BTC when a taker accepts, and completes swaps automatically. It never holds anyone else's funds: the worst a taker can do is make it wait until `t1` and refund.

> [!NOTE]
> v1 has one maker, which we run on testnet. The service binary and its HTTP routes are in progress; the swap runner is done and tested against the engine.

## What it needs

| Component                                        | Why                                                            |
| ------------------------------------------------ | -------------------------------------------------------------- |
| Its own **Zebra** node with light-wallet serving | Bob verifies the ZEC lock only on a node he controls (§4.3-R4) |
| **bitcoind** with a wallet                       | Funds and signs `tx_lock`, broadcasts refunds                  |
| SQLite                                           | Write-ahead swap state and sealed sessions                     |
| A long-term **identity key** (BIP340)            | Signs every quote, so takers and any relay can verify it       |

## The Bob runner

The maker's runner mirrors the engine's. Its ports:

| Port                     | Does                                                 |
| ------------------------ | ---------------------------------------------------- |
| `Wallet`                 | Refund address, fund `tx_lock` as a PSBT, sign it    |
| `ZecNode`                | Lock detection on Bob's own node, sweep after redeem |
| `BtcSource`, `Broadcast` | Bitcoin reads and broadcasts                         |

Safety rules the runner enforces:

* It refuses to broadcast `tx_lock` before Alice's cancel signature and refund adaptor signature are verified (§3.5-R5).
* It checks the wallet didn't change `tx_lock` while signing.
* It saves the lock template and its redeem adaptor signature before sending them.
* After a restart it re-sends its pending messages; the taker accepts identical repeats and refuses conflicting ones.

## Limits

Per-swap and daily caps, a concurrency limit and quote rate limits bound the BTC a griefing taker can tie up (threat T8). See [Network constants](../protocol/network-constants.md).

## Many makers later

The protocol crate knows nothing about transport, quotes are signed by each maker's identity key, and the wire protocol is versioned. That is what lets a quote board or relay list many makers later without being able to forge their quotes.
