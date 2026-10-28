# Welcome to ZecSplit

Swap shielded ZEC for BTC with nobody in the middle who can freeze or steal it.

ZecSplit is a trustless atomic swap between **shielded ZEC** (the Zcash Ironwood pool) and **BTC**. There is no bridge, no custodian, no wrapped coin, and no transparent Zcash address at any point. Either both sides of the trade happen, or neither does.

> [!WARNING]
> **Pre-alpha. Regtest and testnet only. Unaudited.** Do not use ZecSplit with real funds.

Code: [github.com/trudransh/ZecSplit](https://github.com/trudransh/ZecSplit) · App: [zecsplit.vercel.app](https://zecsplit.vercel.app)

## The problem

Shielded ZEC is private while it sits in a wallet. Every common way out of it puts someone in the middle who can stop you or see you:

| Route                 | What goes wrong                                                                                |
| --------------------- | ---------------------------------------------------------------------------------------------- |
| Intent networks       | Your ZEC goes to a fresh **transparent** deposit address, and a compliance hold can freeze it. |
| Centralized exchanges | KYC, freezes, delistings.                                                                      |
| Vault-based protocols | Funds sit in transparent vault addresses run by validators.                                    |

Shielded Zcash has no scripting language, so it can't hold a hash-locked contract the way Bitcoin can. That is why shielded ZEC has had no atomic swaps.

## The idea

ZecSplit ports the Bitcoin–Monero atomic swap (J. Gugger, 2020; in production since 2021 as COMIT's `xmr-btc-swap`, now eigenwallet) to Zcash:

* The ZEC is locked to a fresh shielded address whose spending key is the sum of two secret shares. Alice holds one, Bob the other. Neither can spend alone.
* Every condition (redeem, cancel, refund, punish) lives in **Bitcoin script**.
* Each Bitcoin spending path reveals **exactly one share** to the other party. Whoever learns the missing share can sweep the ZEC.

On Zcash, a swap is two ordinary shielded transactions.

## What a swap guarantees

| Question                                  | Answer                                                                      |
| ----------------------------------------- | --------------------------------------------------------------------------- |
| Can either party end up with both coins?  | No. Every ending leaves each party with its own asset or the other party's. |
| Does the ZEC touch a transparent address? | No. Payment in and sweep out are both shielded.                             |
| Can anyone freeze the swap?               | No. There is no custodian, solver or bridge in the path.                    |
| Is the price fair?                        | Not guaranteed. The signed quote only proves the maker offered it.          |
| Is the Bitcoin side private?              | No more than any Bitcoin transaction.                                       |

## Live on testnet

Three swaps of 0.05 testnet ZEC for 25,000 signet sats: the maker on a cloud VM checking the lock on its own Zebra node, the taker's engine on a laptop with two Bitcoin sources.

| Swap | Maker locks BTC (signet)                                                                                        | Taker pays ZEC (testnet)                                                                                                       | Taker claims BTC (signet)                                                                                       | Maker sweeps ZEC (testnet)                                                                                                     |
| ---- | --------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| 1    | [`dbc65938…`](https://mempool.space/signet/tx/dbc6593847db04dec256aca4dae00c8b5df6a478731d981291be2c7b7937cda1) | [`62cfea06…`](https://testnet.zcashexplorer.app/transactions/62cfea0652d9ec5675db266bcc28f6bac7e63b0ba0f2443b5a1cf03e1e88805c) | [`8968f8f0…`](https://mempool.space/signet/tx/8968f8f08afe3f0bce599b47115f28f42bd0d67ea04f1a92df9137fa525be35d) | [`22451c4e…`](https://testnet.zcashexplorer.app/transactions/22451c4e6e6c8e720ddfec7cc45756b76ed3e9cc95628773c08c3685e94a25e7) |
| 2    | [`224e049f…`](https://mempool.space/signet/tx/224e049f3ba6f7945304e0ed684f8009ccfb1159bf557bb4c7d51873e17af774) | [`2139bd7a…`](https://testnet.zcashexplorer.app/transactions/2139bd7a3a0cb4b24438bca3cf3464d6e83d5eea384fbcd1901114e5f0a8824d) | [`f1569274…`](https://mempool.space/signet/tx/f15692748402e1730c8d36fee1ff3d212057027ac40668381a249f640d2205bd) | [`9125fcb0…`](https://testnet.zcashexplorer.app/transactions/9125fcb06fa15dce4301c17bbbe3b451de0a1769c710d8b1cbaec1f8bfe6a8fc) |
| 3    | [`27024c5d…`](https://mempool.space/signet/tx/27024c5da837cb9ed1dc4eeda2c2f128d4692418e9dea07000f0bdee7af48f9a) | [`abdb747f…`](https://testnet.zcashexplorer.app/transactions/abdb747f0f471fd6a9844940c2db95ef43e68c5362a8a99912ed19f57e189aa6) | [`dce3fce1…`](https://mempool.space/signet/tx/dce3fce1aa550cab3b3e7e2b190b1a2b4388f51104471d375252fcfaf29b3d35) | [`85af11f5…`](https://testnet.zcashexplorer.app/transactions/85af11f506ee0fbf558b2f9d612f7bef1d05d68d027186bb8c02208d528a2c30) |

## Where to start

| If you want to                             | Read                                             |
| ------------------------------------------ | ------------------------------------------------ |
| Run swaps on a local devnet in one command | [Quickstart](quickstart.md)       |
| Understand the idea in ten minutes         | [Core concepts](core-concepts.md) |
| Check the protocol rule by rule            | [Protocol](protocol.md)           |
| See how it was tested and attacked         | [Testing](developers/testing.md)  |
| Know the risks                             | [Security](security.md)           |
| Build on it or run a maker                 | [Developers](developers.md)       |
