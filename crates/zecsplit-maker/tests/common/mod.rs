//! What both runner tests share: the in-process link from the engine to Bob's runner.

use async_trait::async_trait;
use zecsplit_maker::runner::{self as bob, BobRunner};
use zecsplit_protocol::messages::Envelope;
use zecsplit_protocol::recovery::SecretString;
use zecsplit_store::Store;
use zecsplitd::runner::Maker;

/// The maker as the engine reaches it: Bob's runner behind INTERFACES §2.
pub struct Link<'a> {
    pub bob: tokio::sync::Mutex<(BobRunner, Store)>,
    pub ports: bob::Ports<'a>,
}

#[async_trait]
impl Maker for Link<'_> {
    async fn send(&self, msg: &Envelope) -> Result<Option<Envelope>, String> {
        let mut guard = self.bob.lock().await;
        let (runner, store) = &mut *guard;
        runner
            .handle(store, &self.ports, msg.clone(), 0)
            .await
            .map_err(|e| e.to_string())?;
        Ok(runner.next_message())
    }
    async fn next(&self, _: &[u8; 32]) -> Result<Option<Envelope>, String> {
        Ok(self.bob.lock().await.0.next_message())
    }
}

/// The passphrase both test runners seal their sessions with.
pub fn pass() -> SecretString {
    SecretString::from("test passphrase".to_owned())
}
