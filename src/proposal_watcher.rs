use std::fmt::{Debug};
use std::sync::Arc;

use anyhow::Error;
use ethers::prelude::*;
use futures::prelude::*;
use std::time::Duration;
use tracing::Instrument;
use webb::evm::ethers;
use webb::evm::contract::bridge::BridgeContract;

#[derive(Debug, Clone)]
pub struct ProposalWatcher {
    ws_endpoint: String,
    contract: Address,
}

impl ProposalWatcher {

    pub fn new(
        endpoint: impl Into<String>,
        _contract_address: Address,
    ) -> Self {
        Self {
            ws_endpoint: endpoint.into(),
            contract: _contract_address,
        }
    }

    #[tracing::instrument(skip(self))]
    pub async fn run(&self) -> anyhow::Result<()> {
        let backoff = backoff::ExponentialBackoff {
            max_elapsed_time: None,
            ..Default::default()
        };
        let task = || async { self.watch().await };
        backoff::future::retry(backoff, task).await?;
        Ok(())
    }

    async fn watch(&self) -> anyhow::Result<(), backoff::Error<Error>> {
        tracing::trace!("Connecting to {} for proposals", self.ws_endpoint);
        let endpoint = url::Url::parse(&self.ws_endpoint)
            .map_err(Error::from)
            .map_err(backoff::Error::Permanent)?;
        let ws = Ws::connect(endpoint)
            .map_err(Error::from)
            .instrument(tracing::trace_span!("websocket"))
            .await?;
        let fetch_interval = Duration::from_millis(200);
        let provider = Provider::new(ws).interval(fetch_interval);
        let client = Arc::new(provider);
        self.poll_for_proposals(client).await?;
        Ok(())
    }

    async fn poll_for_proposals(&self, client: Arc<Provider<Ws>>) -> Result<(), backoff::Error<Error>> {
        // now we start polling for new events.
        let mut block = client.get_block_number().map_err(Error::from).await?;
        let contract = BridgeContract::new(self.contract, client.clone());

        loop {
            let current_block_number = client.get_block_number().map_err(Error::from).await?;
            let events_filter = contract.proposal_event_filter()
                .from_block(block)
                .to_block(current_block_number);
            let found_events = events_filter.query_with_meta().map_err(Error::from).await?;

            tracing::trace!("Found #{} proposals", found_events.len());

            tracing::trace!("Polled from #{} to #{}", block, current_block_number);

            block = current_block_number;

            tracing::trace!("Current block number (proposals): #{}", current_block_number);

            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    }

}


