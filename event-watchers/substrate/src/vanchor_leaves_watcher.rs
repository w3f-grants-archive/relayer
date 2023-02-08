// Copyright 2022 Webb Technologies Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::sync::Arc;
use tokio::sync::Mutex;
use webb::substrate::protocol_substrate_runtime;
use webb::substrate::protocol_substrate_runtime::api as RuntimeApi;
use webb::substrate::protocol_substrate_runtime::api::v_anchor_bn254;
use webb::substrate::subxt::{self, OnlineClient};
use webb_event_watcher_traits::substrate::BlockNumberOf;
use webb_event_watcher_traits::SubstrateEventWatcher;
use webb_proposals::{
    ResourceId, SubstrateTargetSystem, TargetSystem, TypedChainId,
};
use webb_relayer_store::sled::SledStore;
use webb_relayer_store::LeafCacheStore;
use webb_relayer_utils::metric;
// An Substrate VAnchor Leaves Watcher that watches for Deposit events and save the leaves to the store.
/// It serves as a cache for leaves that could be used by dApp for proof generation.
#[derive(Clone, Debug, Default)]
pub struct SubstrateVAnchorLeavesWatcher;

#[async_trait::async_trait]
impl SubstrateEventWatcher for SubstrateVAnchorLeavesWatcher {
    const TAG: &'static str = "Substrate V-Anchor leaves watcher";

    const PALLET_NAME: &'static str = "VAnchorBn254";

    type RuntimeConfig = subxt::SubstrateConfig;

    type Client = OnlineClient<Self::RuntimeConfig>;

    type Event = protocol_substrate_runtime::api::Event;

    type FilteredEvent = v_anchor_bn254::events::Transaction;

    type Store = SledStore;

    async fn handle_event(
        &self,
        store: Arc<Self::Store>,
        api: Arc<Self::Client>,
        (event, block_number): (Self::FilteredEvent, BlockNumberOf<Self>),
        _metrics: Arc<Mutex<metric::Metrics>>,
    ) -> webb_relayer_utils::Result<()> {
        let at_hash_addr = RuntimeApi::storage()
            .system()
            .block_hash(block_number as u64);
        let at_hash = api.storage().fetch(&at_hash_addr, None).await?.unwrap();
        // fetch leaf_index from merkle tree at given block_number
        let next_leaf_index_addr = RuntimeApi::storage()
            .merkle_tree_bn254()
            .next_leaf_index(event.tree_id);
        let next_leaf_index = api
            .storage()
            .fetch(&next_leaf_index_addr, Some(at_hash))
            .await?
            .unwrap();
        // fetch chain_id
        let chain_id_addr = RuntimeApi::constants()
            .linkable_tree_bn254()
            .chain_identifier();
        let chain_id = api.constants().at(&chain_id_addr)?;
        let tree_id = event.tree_id.to_string();
        let leaf_count = event.leafs.len();

        // pallet index
        let pallet_index = {
            let metadata = api.metadata();
            let pallet = metadata.pallet("VAnchorHandlerBn254")?;
            pallet.index()
        };
        let src_chain_id = TypedChainId::Substrate(chain_id as u32);
        let target = SubstrateTargetSystem::builder()
            .pallet_index(pallet_index)
            .tree_id(event.tree_id)
            .build();
        let src_target_system = TargetSystem::Substrate(target);
        let history_store_key =
            ResourceId::new(src_target_system, src_chain_id);
        let mut leaf_index = next_leaf_index.saturating_sub(leaf_count as u32);
        let mut leaf_store = Vec::with_capacity(leaf_count);
        for leaf in event.leafs {
            let value = (leaf_index, leaf.0.to_vec());
            store.insert_leaves(history_store_key, &[value])?;
            store.insert_last_deposit_block_number(
                history_store_key,
                block_number.into(),
            )?;
            leaf_index += 1;
            leaf_store.push(leaf.0);
        }
        tracing::event!(
            target: webb_relayer_utils::probe::TARGET,
            tracing::Level::DEBUG,
            kind = %webb_relayer_utils::probe::Kind::LeavesStore,
            chain_id = %chain_id,
            leaf_index = leaf_index,
            leafs = %format!("{leaf_store:?}"),
            tree_id = %tree_id,
            block_number = %block_number
        );
        Ok(())
    }
}