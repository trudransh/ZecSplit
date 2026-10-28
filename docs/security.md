# Security

What ZecSplit protects, who might attack it, and what is left.

> [!CAUTION]
> ZecSplit is **unaudited** pre-alpha software. It runs on regtest and testnet only.

## Atomicity

In every terminal state, each party holds either its original asset or the counterparty's:

| Ending                  | Alice                             | Bob                                                                |
| ----------------------- | --------------------------------- | ------------------------------------------------------------------ |
| Redeem                  | Gets the BTC                      | Learns `s_a`, sweeps the ZEC                                       |
| Refund                  | Learns `s_b`, sweeps her ZEC back | Gets his BTC back                                                  |
| Punish                  | Gets the BTC                      | Loses the BTC; the ZEC stays locked. The penalty for going silent. |
| Abort (before any lock) | Keeps her ZEC                     | Keeps his BTC                                                      |

## The rules that carry the security

Reviews check these first:

* **Commit before reveal.** Neither party reveals a share before holding the other's commitment, so nobody can pick a share as a function of the other's (§3.1-R2).
* **Bob locks only once he is protected.** Bob never broadcasts `tx_lock` before holding Alice's cancel signature and refund adaptor signature (§3.5-R5).
* **Alice pays only on a confirmed lock.** The payment request appears only after two independent Bitcoin sources confirm `tx_lock` (§4.2).
* **The redeem margin.** Alice never reveals `s_a` close to the cancel deadline (§4.4-R1).
* **Write-ahead.** Every state change is saved before the message or broadcast it allows (§6.3-R1).

## Pages

* [Threat model](security/threat-model.md): 25 threats, their mitigations and their tests.
* [Privacy](security/privacy.md): what each observer learns.

## Assumptions we state publicly

1. The ECDSA adaptor signatures (`ecdsa_fun`) and the cross-group DLEq (Serai `dleq`) are sound as implemented upstream.
2. Bitcoin consensus enforces BIP68 and BIP143. Zcash consensus enforces Ironwood spend-authorization signatures.
3. Bob runs his own nodes. Alice uses two independent Bitcoin sources.
4. **Users come back online between `t1` and `t1 + t2` if something goes wrong.** This is the biggest usability assumption; a watchtower is planned to remove it.
5. The users' machines are not compromised.

## Residual risks

* **No audit.**
* **Liveness.** A user who stays offline past `t1 + t2` while the maker misbehaves can lose funds.
* **Fee markets** can make the redeem-vs-cancel and refund-vs-punish races tighter than modelled.
