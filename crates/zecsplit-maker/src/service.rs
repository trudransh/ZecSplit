//! The maker service: signs quotes, keeps one Bob runner per swap, and ticks them all (INTERFACES §2).

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use zecsplit_crypto::rand_core::{OsRng, RngCore};
use zecsplit_crypto::{MakerSecret, Role, maker_public_key};
use zecsplit_protocol::messages::{
    Envelope, ErrorBody, Message, Network, PROTOCOL_VERSION, Quote, QuoteRequest, SignedQuote,
};
use zecsplit_protocol::recovery::SecretString;
use zecsplit_protocol::session::Core;
use zecsplit_protocol::session::bob::BobSession;
use zecsplit_store::Store;

use crate::runner::{BobRunner, Ports, RunnerError};

/// What the operator configures.
pub struct Config {
    pub network: Network,
    pub sats_per_zec: u64,
    pub min_zec: u64,
    pub max_zec: u64,
    pub btc_fee_rate: u64,
    pub endpoint: String,
    pub passphrase: SecretString,
}

/// §8: timelocks and confirmations per network, as `(t1, t2, n_conf_btc, n_conf_zec, redeem_margin)`.
fn timings(network: Network) -> Option<(u16, u16, u16, u16, u16)> {
    match network {
        Network::Regtest => Some((20, 20, 1, 1, 5)),
        Network::Test => Some((36, 36, 1, 5, 12)),
        Network::MainBeta => None,
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

fn error(swap_id: [u8; 32], code: &str, message: String) -> Envelope {
    Envelope {
        v: PROTOCOL_VERSION,
        swap_id,
        message: Message::Error(ErrorBody {
            code: code.to_string(),
            message,
        }),
    }
}

/// One maker: its identity, its swaps and everything they talk to.
pub struct Service {
    config: Config,
    identity: MakerSecret,
    store: Store,
    runners: HashMap<[u8; 32], BobRunner>,
    ports: Ports<'static>,
}

impl Service {
    /// §6.3-R2: opens the store and resumes every unfinished swap.
    pub fn open(
        config: Config,
        identity: MakerSecret,
        mut store: Store,
        ports: Ports<'static>,
    ) -> Result<Self, RunnerError> {
        let mut runners = HashMap::new();
        for row in store.active_swaps()? {
            if let Some(r) = BobRunner::resume(&mut store, &row, config.passphrase.clone(), now())?
            {
                runners.insert(row.id, r);
            }
        }
        Ok(Service {
            config,
            identity,
            store,
            runners,
            ports,
        })
    }

    /// §2-R1: the key takers pin.
    pub fn identity_key(&self) -> Result<[u8; 32], String> {
        maker_public_key(&self.identity).map_err(|e| e.to_string())
    }

    /// §2-R2: a signed 60 s quote within the limits, with its swap recorded before it leaves.
    pub async fn quote(&mut self, req: &QuoteRequest) -> Envelope {
        match self.try_quote(req).await {
            Ok(q) => Envelope {
                v: PROTOCOL_VERSION,
                swap_id: q.quote.swap_id,
                message: Message::Quote(q),
            },
            Err(e) => error([0; 32], "quote_refused", e),
        }
    }

    async fn try_quote(&mut self, req: &QuoteRequest) -> Result<SignedQuote, String> {
        if req.network != self.config.network {
            return Err("network not served".into());
        }
        if req.zec_amount < self.config.min_zec || req.zec_amount > self.config.max_zec {
            return Err("amount outside the limits".into());
        }
        let (t1, t2, n_conf_btc, n_conf_zec, redeem_margin) =
            timings(req.network).ok_or("network not served")?;
        let btc_amount = u64::try_from(
            u128::from(req.zec_amount) * u128::from(self.config.sats_per_zec) / 100_000_000,
        )
        .map_err(|e| e.to_string())?;
        let mut swap_id = [0; 32];
        OsRng.fill_bytes(&mut swap_id);
        let quote = Quote {
            protocol_version: PROTOCOL_VERSION,
            swap_id,
            network: req.network,
            zec_amount: req.zec_amount,
            btc_amount,
            btc_fee_rate: self.config.btc_fee_rate,
            t1,
            t2,
            n_conf_btc,
            n_conf_zec,
            redeem_margin,
            expires_at: u64::try_from(now()).map_err(|e| e.to_string())? + 60,
        };
        let refund = self.ports.wallet.refund_address().await?;
        let core =
            Core::new(&mut OsRng, quote.clone(), Role::Bob, refund).map_err(|e| e.to_string())?;
        let runner = BobRunner::start(
            &mut self.store,
            BobSession::new(core),
            self.config.passphrase.clone(),
            &self.config.endpoint,
            now(),
        )
        .map_err(|e| e.to_string())?;
        self.runners.insert(swap_id, runner);
        quote.sign(&self.identity).map_err(|e| e.to_string())
    }

    /// INTERFACES §2 `POST …/msg`: handles one taker message and returns the next reply, if any.
    pub async fn message(&mut self, env: Envelope) -> Option<Envelope> {
        let id = env.swap_id;
        let Some(runner) = self.runners.get_mut(&id) else {
            return Some(error(id, "unknown_swap", "no such swap".into()));
        };
        match runner
            .handle(&mut self.store, &self.ports, env, now())
            .await
        {
            Ok(()) => runner.next_message(),
            Err(e) => Some(error(id, "maker_failed", e.to_string())),
        }
    }

    /// INTERFACES §2 `GET …/next`: the next pending message for `id`.
    pub fn next(&mut self, id: &[u8; 32]) -> Option<Envelope> {
        self.runners.get_mut(id).and_then(BobRunner::next_message)
    }

    /// One tick of every unfinished swap; a failing swap is reported and the others still run.
    pub async fn tick(&mut self) -> Vec<String> {
        let mut failures = Vec::new();
        for (id, runner) in self.runners.iter_mut() {
            if runner.state.is_terminal() {
                continue;
            }
            if let Err(e) = runner.tick(&mut self.store, &self.ports, now()).await {
                failures.push(format!("{}: {e}", hex::encode(id)));
            }
        }
        failures
    }
}
