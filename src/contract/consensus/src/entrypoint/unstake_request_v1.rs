/* This file is part of DarkFi (https://dark.fi)
 *
 * Copyright (C) 2020-2024 Dyne.org foundation
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as
 * published by the Free Software Foundation, either version 3 of the
 * License, or (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU Affero General Public License for more details.
 *
 * You should have received a copy of the GNU Affero General Public License
 * along with this program.  If not, see <https://www.gnu.org/licenses/>.
 */

use darkfi_money_contract::{
    error::MoneyError, model::ConsensusUnstakeReqParamsV1, CONSENSUS_CONTRACT_INFO_TREE,
    CONSENSUS_CONTRACT_NULLIFIERS_TREE, CONSENSUS_CONTRACT_STAKED_COIN_ROOTS_TREE,
    CONSENSUS_CONTRACT_UNSTAKED_COINS_TREE, CONSENSUS_CONTRACT_UNSTAKED_COIN_LATEST_COIN_ROOT,
    CONSENSUS_CONTRACT_UNSTAKED_COIN_MERKLE_TREE, CONSENSUS_CONTRACT_UNSTAKED_COIN_ROOTS_TREE,
    CONSENSUS_CONTRACT_ZKAS_BURN_NS_V1, CONSENSUS_CONTRACT_ZKAS_MINT_NS_V1,
};
use darkfi_sdk::{
    crypto::{pasta_prelude::*, ContractId, MerkleNode},
    dark_tree::DarkLeaf,
    db::{db_contains_key, db_lookup, db_set},
    error::{ContractError, ContractResult},
    merkle_add, msg,
    pasta::pallas,
    util::get_verifying_slot_epoch,
    ContractCall,
};
use darkfi_serial::{deserialize, serialize, Encodable, WriteExt};

use crate::{
    error::ConsensusError,
    model::{ConsensusProposalUpdateV1, GRACE_PERIOD},
    ConsensusFunction,
};

/// `get_metadata` function for `Consensus::UnstakeRequestV1`
pub(crate) fn consensus_unstake_request_get_metadata_v1(
    _cid: ContractId,
    call_idx: u32,
    calls: Vec<DarkLeaf<ContractCall>>,
) -> Result<Vec<u8>, ContractError> {
    let self_ = &calls[call_idx as usize].data;
    let params: ConsensusUnstakeReqParamsV1 = deserialize(&self_.data[1..])?;
    let input = &params.input;
    let output = &params.output;

    // Public inputs for the ZK proofs we have to verify
    let mut zk_public_inputs: Vec<(String, Vec<pallas::Base>)> = vec![];
    // Public keys for the transaction signatures we have to verify
    let signature_pubkeys = vec![input.signature_public];

    // Grab the pedersen commitments and signature pubkeys from the
    // anonymous input
    let value_coords = input.value_commit.to_affine().coordinates().unwrap();
    let (sig_x, sig_y) = input.signature_public.xy();

    // It is very important that these are in the same order as the
    // `constrain_instance` calls in the zkas code.
    // Otherwise verification will fail.
    zk_public_inputs.push((
        CONSENSUS_CONTRACT_ZKAS_BURN_NS_V1.to_string(),
        vec![
            input.nullifier.inner(),
            input.epoch.into(),
            sig_x,
            sig_y,
            input.merkle_root.inner(),
            *value_coords.x(),
            *value_coords.y(),
        ],
    ));

    // Grab the minting epoch of the verifying slot
    let epoch = get_verifying_slot_epoch();

    // Grab the pedersen commitment from the anonymous output
    let value_coords = output.value_commit.to_affine().coordinates().unwrap();

    zk_public_inputs.push((
        CONSENSUS_CONTRACT_ZKAS_MINT_NS_V1.to_string(),
        vec![epoch.into(), output.coin.inner(), *value_coords.x(), *value_coords.y()],
    ));

    // Serialize everything gathered and return it
    let mut metadata = vec![];
    zk_public_inputs.encode(&mut metadata)?;
    signature_pubkeys.encode(&mut metadata)?;

    Ok(metadata)
}

/// `process_instruction` function for `Consensus::UnstakeRequestV1`
pub(crate) fn consensus_unstake_request_process_instruction_v1(
    cid: ContractId,
    call_idx: u32,
    calls: Vec<DarkLeaf<ContractCall>>,
) -> Result<Vec<u8>, ContractError> {
    let self_ = &calls[call_idx as usize].data;
    let params: ConsensusUnstakeReqParamsV1 = deserialize(&self_.data[1..])?;
    let input = &params.input;
    let output = &params.output;

    // Access the necessary databases where there is information to
    // validate this state transition.
    let nullifiers_db = db_lookup(cid, CONSENSUS_CONTRACT_NULLIFIERS_TREE)?;
    let unstaked_coins_db = db_lookup(cid, CONSENSUS_CONTRACT_UNSTAKED_COINS_TREE)?;
    let staked_coins_roots_db = db_lookup(cid, CONSENSUS_CONTRACT_STAKED_COIN_ROOTS_TREE)?;

    // ===================================
    // Perform the actual state transition
    // ===================================

    msg!("[ConsensusUnstakeRequestV1] Validating anonymous input");

    // The coin has passed through the grace period and is allowed to request unstake.
    if input.epoch != 0 && get_verifying_slot_epoch() - input.epoch <= GRACE_PERIOD {
        msg!("[ConsensusUnstakeRequestV1] Error: Coin is not allowed to request unstake yet");
        return Err(ConsensusError::CoinStillInGracePeriod.into())
    }

    // The Merkle root is used to know whether this is a coin that
    // existed in a previous state.
    if !db_contains_key(staked_coins_roots_db, &serialize(&input.merkle_root))? {
        msg!("[ConsensusUnstakeRequestV1] Error: Merkle root not found in previous state");
        return Err(MoneyError::TransferMerkleRootNotFound.into())
    }

    // The nullifiers should not already exist. It is the double-spend protection.
    if db_contains_key(nullifiers_db, &serialize(&input.nullifier))? {
        msg!("[ConsensusUnstakeRequestV1] Error: Duplicate nullifier found");
        return Err(MoneyError::DuplicateNullifier.into())
    }

    msg!("[ConsensusUnstakeRequestV1] Validating anonymous output");

    // Verify value commits match
    if output.value_commit != input.value_commit {
        msg!("[ConsensusUnstakeRequestV1] Error: Value commitments do not match");
        return Err(MoneyError::ValueMismatch.into())
    }

    // Newly created coin for this call is in the output. Here we gather it,
    // and we also check that it hasn't existed before.
    if db_contains_key(unstaked_coins_db, &serialize(&output.coin))? {
        msg!("[ConsensusUnstakeRequestV1] Error: Duplicate coin found in output");
        return Err(MoneyError::DuplicateCoin.into())
    }

    // At this point the state transition has passed, so we create a state update
    let update = ConsensusProposalUpdateV1 { nullifier: input.nullifier, coin: output.coin };
    let mut update_data = vec![];
    update_data.write_u8(ConsensusFunction::UnstakeRequestV1 as u8)?;
    update.encode(&mut update_data)?;
    Ok(update_data)
}

/// `process_update` function for `Consensus::UnstakeRequestV1`
pub(crate) fn consensus_unstake_request_process_update_v1(
    cid: ContractId,
    update: ConsensusProposalUpdateV1,
) -> ContractResult {
    // Grab all necessary db handles for where we want to write
    let info_db = db_lookup(cid, CONSENSUS_CONTRACT_INFO_TREE)?;
    let nullifiers_db = db_lookup(cid, CONSENSUS_CONTRACT_NULLIFIERS_TREE)?;
    let unstaked_coins_db = db_lookup(cid, CONSENSUS_CONTRACT_UNSTAKED_COINS_TREE)?;
    let unstaked_coin_roots_db = db_lookup(cid, CONSENSUS_CONTRACT_UNSTAKED_COIN_ROOTS_TREE)?;

    msg!("[ConsensusUnstakeRequestV1] Adding new nullifier to the set");
    db_set(nullifiers_db, &serialize(&update.nullifier), &[])?;

    msg!("[ConsensusUnstakeRequestV1] Adding new coin to the unstaked coins set");
    db_set(unstaked_coins_db, &serialize(&update.coin), &[])?;

    msg!("[ConsensusUnstakeRequestV1] Adding new coin to the unstaked coins Merkle tree");
    let coins: Vec<_> = vec![MerkleNode::from(update.coin.inner())];
    merkle_add(
        info_db,
        unstaked_coin_roots_db,
        CONSENSUS_CONTRACT_UNSTAKED_COIN_LATEST_COIN_ROOT,
        CONSENSUS_CONTRACT_UNSTAKED_COIN_MERKLE_TREE,
        &coins,
    )?;

    Ok(())
}
