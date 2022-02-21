use std::convert::TryFrom;
use std::marker::PhantomData;
use std::ops;
use std::sync::Arc;
use std::time::Duration;

use webb::evm::contract::protocol_solidity::{
    AnchorHandlerContract, FixedDepositAnchorContract,
    FixedDepositAnchorContractEvents, SignatureBridgeContract
};

use webb::evm::ethers::prelude::*;
use webb::evm::ethers::prelude::k256::{SecretKey};
use webb::evm::ethers::providers;
use webb::evm::ethers::types;
use webb::evm::ethers::utils::keccak256;

use crate::config;
use crate::store::sled::SledStore;
use crate::store::LeafCacheStore;
use crate::events_watcher::{
    encode_resource_id, ProposalData, ProposalHeader, ProposalNonce,
};

type HttpProvider = providers::Provider<providers::Http>;

pub struct ForOracle;

#[derive(Copy, Clone, Debug)]
pub struct AnchorWatcher<H>(PhantomData<H>);

impl<H> AnchorWatcher<H> {
    pub const fn new() -> AnchorWatcher<H> {
        Self(PhantomData)
    }
}

pub type AnchorOracleWatcher = AnchorWatcher<ForOracle>;

#[derive(Clone, Debug)]
pub struct AnchorOracleContractWrapper<M: Middleware> {
    config: config::AnchorContractOracleConfig,
    webb_config: config::WebbRelayerConfig,
    contract: FixedDepositAnchorContract<M>,
}

impl<M: Middleware> AnchorOracleContractWrapper<M> {
    pub fn new(
        config: config::AnchorContractOracleConfig,
        webb_config: config::WebbRelayerConfig,
        client: Arc<M>,
    ) -> Self {
        Self {
            contract: FixedDepositAnchorContract::new(
                config.common.address,
                client,
            ),
            config,
            webb_config,
        }
    }

    // pub fn signMessage(
    //     &self,
    //     data: &[u8],
    //     key: &str,
    // ) -> Vec<u8> {
    //     // hash the data to sign
    //     let hashed_data = keccak256(data);
    //     let key = SecretKey::from_bytes(self.webb_config..as_bytes())?;
    //     let chain_id = chain_config.chain_id;
    //     let wallet = LocalWallet::from(key).with_chain_id(chain_id);

    //     // self.provider().sign(hashed_data, )
    //     let signature = signing_key.sign(hashed_data).unwrap();

    //     signature.to_vec()
    // }
}

impl<M: Middleware> ops::Deref for AnchorOracleContractWrapper<M> {
    type Target = Contract<M>;

    fn deref(&self) -> &Self::Target {
        &self.contract
    }
}

impl<M: Middleware> super::WatchableContract for AnchorOracleContractWrapper<M> {
    fn deployed_at(&self) -> types::U64 {
        self.config.common.deployed_at.into()
    }

    fn polling_interval(&self) -> Duration {
        Duration::from_millis(self.config.events_watcher.polling_interval)
    }

    fn max_events_per_step(&self) -> types::U64 {
        self.config.events_watcher.max_events_per_step.into()
    }

    fn print_progress_interval(&self) -> Duration {
        Duration::from_millis(
            self.config.events_watcher.print_progress_interval,
        )
    }
}

#[async_trait::async_trait]
impl super::EventWatcher for AnchorWatcher<ForOracle> {
    const TAG: &'static str = "Anchor Watcher For Oracle";
    type Middleware = HttpProvider;

    type Contract = AnchorOracleContractWrapper<Self::Middleware>;

    type Events = FixedDepositAnchorContractEvents;

    type Store = SledStore;

    #[tracing::instrument(skip_all)]
    async fn handle_event(
        &self,
        store: Arc<Self::Store>,
        wrapper: &Self::Contract,
        (event, log): (Self::Events, LogMeta),
    ) -> anyhow::Result<()> {
        use FixedDepositAnchorContractEvents::*;
        // only process anchor deposit events.
        let event_data = match event {
            DepositFilter(data) => {
                let commitment = data.commitment;
                let leaf_index = data.leaf_index;
                let value = (leaf_index, H256::from_slice(&commitment));
                let chain_id = wrapper.contract.client().get_chainid().await?;
                store.insert_leaves(
                    (chain_id, wrapper.contract.address()),
                    &[value],
                )?;
                store.insert_last_deposit_block_number(
                    (chain_id, wrapper.contract.address()),
                    log.block_number,
                )?;
                
                tracing::trace!(
                    "detected log.block_number: {}",
                    log.block_number
                );
                tracing::debug!(
                    "Saved Deposit Event ({}, {}) at block {}",
                    value.0,
                    value.1,
                    log.block_number
                );
                data
            },
            _ => return Ok(()),
        };
        let client = wrapper.contract.client();
        let src_chain_id = client.get_chainid().await?;
        let root = wrapper.contract.get_last_root().call().await?;
        let leaf_index = event_data.leaf_index;
        // the correct way for getting the other linked anchors
        // is by getting it from the edge_list, but for now we hardcoded
        // them in the config.

        // **The Signaling Flow**
        //
        // For Every Linked Anchor, we do the following:
        // 1. Get the chain information of that anchor from the config,
        //    if not found, we skip (we should print a warning here).
        // 2. We call that chain `dest_chain`, then we create a connection to that
        //    dest_chain, which we will construct the other linked anchor contract
        //    to query the following information:
        //      a. dest_chain_id (the chain_id of that linked anchor).
        //      b. dest_bridge (the bridge of that linked anchor on the other chain).
        //      c. dest_handler (the address of the handler that linked to that anchor).
        // 3. Then we create a `BridgeKey` of that `dest_bridge` to send the proposal data.
        //    if not found, we skip.
        // 4. Signal the bridge with the following data:
        //      a. dest_contract (the Anchor contract on dest_chain).
        //      b. dest_handler (used for creating data_hash).
        //      c. origin_chain_id (used for creating proposal).
        //      d. leaf_index (used as nonce, for creating proposal).
        //      e. merkle_root (the new merkle_root, used for creating proposal).
        //
        'outer: for linked_anchor in &wrapper.config.linked_anchors {
            let dest_chain = linked_anchor.chain.to_lowercase();
            let maybe_chain = wrapper.webb_config.evm.get(&dest_chain);
            let dest_chain = match maybe_chain {
                Some(chain) => chain,
                None => continue,
            };
            // TODO(@shekohex): store clients in connection pool, so don't
            // have to create a new connection every time.
            let provider =
                HttpProvider::try_from(dest_chain.http_endpoint.as_str())?
                    .interval(Duration::from_millis(6u64));
            let dest_client = Arc::new(provider);
            let dest_chain_id = dest_client.get_chainid().await?;
            let dest_anchor = FixedDepositAnchorContract::new(
                linked_anchor.address,
                dest_client.clone(),
            );
            let experimental = wrapper.webb_config.experimental;
            let retry_count = if experimental.smart_anchor_updates {
                // this just a sane number of retries, before we actually issue the update proposal
                experimental.smart_anchor_updates_retries
            } else {
                // if this is not an experimental smart anchor, we don't need to retry
                // hence we skip the whole smart logic here.
                0
            };
            for _ in 0..retry_count {
                // we are going to query for the latest leaf index of the dest_chain
                let dest_leaf_index = dest_anchor.next_index().call().await?;
                // now we compare this leaf index with the leaf index of the origin chain
                // if the leaf index is greater than the leaf index of the origin chain,
                // we skip this linked anchor.
                if leaf_index < dest_leaf_index.saturating_sub(1) {
                    tracing::debug!(
                        "skipping linked anchor {} because leaf index {} is less than {}",
                        linked_anchor.address,
                        leaf_index,
                        dest_leaf_index.saturating_sub(1)
                    );
                    // continue on the next anchor, from the outer loop.
                    continue 'outer;
                }
                // if the leaf index is less than the leaf index of the origin chain,
                // we should do the following:
                // 1. sleep for a 10s to 30s (max).
                // 2. re-query the leaf index of the dest chain.
                // 3. if the leaf index is greater than the leaf index of the origin chain,
                //    we skip this linked anchor.
                // 4. if the leaf index is less than the leaf index of the origin chain,
                //    we will continue to retry again for `retry_count`.
                // 5. at the end, we will issue the proposal to the bridge.
                let s = 10;
                tracing::debug!("sleep for {}s before signaling the bridge", s);
                tokio::time::sleep(Duration::from_secs(s)).await;
            }
            // to get the bridge address, we need to get the anchor handler address first, and from there
            // we can get the bridge address.
            let dest_handler = dest_anchor.handler().call().await?;
            let dest_handler_contract =
                AnchorHandlerContract::new(dest_handler, dest_client.clone());
            let dest_bridge_address =
                dest_handler_contract.bridge_address().call().await?;

            let function_sig = dest_anchor
                .update_edge(src_chain_id, root, types::U256::from(leaf_index))
                .function
                .short_signature();
            let data = ProposalData {
                anchor_address: dest_anchor.address(),
                anchor_handler_address: dest_handler,
                src_chain_id,
                leaf_index,
                function_sig,
                merkle_root: root,
            };
            let mut proposal_data = Vec::with_capacity(82);
            let resource_id =
                encode_resource_id(data.anchor_address, [1, 0], dest_chain_id)?;
            let header = ProposalHeader {
                resource_id,
                function_sig: data.function_sig,
                chain_id: dest_chain_id.as_u32(),
                nonce: ProposalNonce::from(data.leaf_index)+1,
            };
            // first the header (40 bytes)
            header.encoded_to(&mut proposal_data);
            // next, the origin chain type (2 bytes)
            proposal_data.extend_from_slice(&[1, 0]);
            // next, the origin chain id (4 bytes)
            proposal_data
                .extend_from_slice(&data.src_chain_id.as_u32().to_be_bytes());
            // next, the leaf index (4 bytes)
            proposal_data.extend_from_slice(&data.leaf_index.to_be_bytes());
            // next, the merkle root (32 bytes)
            proposal_data.extend_from_slice(&data.merkle_root);

            // Sign data and update the other side of the bridge
            tracing::debug!(
                "Detected bridge side address as: {}",
                dest_bridge_address
            );
            let dest_bridge_side = SignatureBridgeContract::new(dest_bridge_address, dest_client.clone());

            // build up the wallet for signing
            let key = SecretKey::from_bytes(dest_chain.private_key.as_bytes())?;
            let chain_id = dest_chain.chain_id;
            let wallet = LocalWallet::from(key).with_chain_id(chain_id);

            // hash the data to sign
            let hashed_data = H256::from(keccak256(Bytes::from(proposal_data.clone())));

            tracing::debug!(
                "formatted proposal: {:?}",
                proposal_data
            );

            let signed_data = wallet.sign_hash(hashed_data, false);
            let formatted_data = signed_data.to_vec();

            tracing::debug!(
                "formatted data of signature: {:?}",
                formatted_data
            );

            dest_bridge_side.execute_proposal_with_signature(Bytes::from(hashed_data.as_bytes().to_vec()), Bytes::from(formatted_data)).call().await?;
        }
        Ok(())
    }
}
