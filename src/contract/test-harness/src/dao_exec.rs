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

use std::time::Instant;

use darkfi::{tx::Transaction, Result};
use darkfi_dao_contract::{
    client::{DaoAuthMoneyTransferCall, DaoExecCall},
    model::{Dao, DaoBulla, DaoExecParams, DaoProposal},
    DaoFunction, DAO_CONTRACT_ZKAS_DAO_AUTH_MONEY_TRANSFER_ENC_COIN_NS,
    DAO_CONTRACT_ZKAS_DAO_AUTH_MONEY_TRANSFER_NS, DAO_CONTRACT_ZKAS_DAO_EXEC_NS,
};
use darkfi_money_contract::{
    client::transfer_v1 as xfer,
    model::{CoinAttributes, MoneyTransferParamsV1},
    MoneyFunction, MONEY_CONTRACT_ZKAS_BURN_NS_V1, MONEY_CONTRACT_ZKAS_MINT_NS_V1,
};
use darkfi_sdk::{
    crypto::{
        pasta_prelude::Field, pedersen_commitment_u64, MerkleNode, SecretKey, DAO_CONTRACT_ID,
        MONEY_CONTRACT_ID,
    },
    dark_tree::DarkLeaf,
    pasta::pallas,
    ContractCall,
};
use darkfi_serial::{serialize, Encodable};
use rand::rngs::OsRng;

use super::{Holder, TestHarness, TxAction};

impl TestHarness {
    #[allow(clippy::too_many_arguments)]
    pub fn dao_exec(
        &mut self,
        dao: &Dao,
        dao_bulla: &DaoBulla,
        proposal: &DaoProposal,
        proposal_coinattrs: Vec<CoinAttributes>,
        yes_vote_value: u64,
        all_vote_value: u64,
        yes_vote_blind: pallas::Scalar,
        all_vote_blind: pallas::Scalar,
    ) -> Result<(Transaction, MoneyTransferParamsV1, DaoExecParams)> {
        let dao_wallet = self.holders.get(&Holder::Dao).unwrap();

        let (mint_pk, mint_zkbin) =
            self.proving_keys.get(&MONEY_CONTRACT_ZKAS_MINT_NS_V1.to_string()).unwrap();
        let (burn_pk, burn_zkbin) =
            self.proving_keys.get(&MONEY_CONTRACT_ZKAS_BURN_NS_V1.to_string()).unwrap();
        let (dao_exec_pk, dao_exec_zkbin) =
            self.proving_keys.get(&DAO_CONTRACT_ZKAS_DAO_EXEC_NS.to_string()).unwrap();
        let (dao_auth_xfer_pk, dao_auth_xfer_zkbin) = self
            .proving_keys
            .get(&DAO_CONTRACT_ZKAS_DAO_AUTH_MONEY_TRANSFER_NS.to_string())
            .unwrap();
        let (dao_auth_xfer_enc_coin_pk, dao_auth_xfer_enc_coin_zkbin) = self
            .proving_keys
            .get(&DAO_CONTRACT_ZKAS_DAO_AUTH_MONEY_TRANSFER_ENC_COIN_NS.to_string())
            .unwrap();

        let tx_action_benchmark = self.tx_action_benchmarks.get_mut(&TxAction::DaoExec).unwrap();
        let timer = Instant::now();

        let input_user_data_blind = pallas::Base::random(&mut OsRng);
        // TODO: FIXME: This is not checked anywhere!
        let exec_signature_secret = SecretKey::random(&mut OsRng);

        assert!(!proposal_coinattrs.is_empty());
        let proposal_token_id = proposal_coinattrs[0].token_id;
        assert!(proposal_coinattrs.iter().all(|c| c.token_id == proposal_token_id));
        let proposal_amount = proposal_coinattrs.iter().map(|c| c.value).sum();

        let dao_coins = dao_wallet
            .unspent_money_coins
            .iter()
            .filter(|x| x.note.token_id == proposal_token_id)
            .cloned()
            .collect();
        let (spent_coins, change_value) = xfer::select_coins(dao_coins, proposal_amount)?;
        let tree = dao_wallet.money_merkle_tree.clone();

        let mut inputs = vec![];
        for coin in &spent_coins {
            let leaf_position = coin.leaf_position;
            let merkle_path = tree.witness(leaf_position, 0).unwrap();

            inputs.push(xfer::TransferCallInput {
                leaf_position,
                merkle_path,
                secret: coin.secret,
                note: coin.note.clone(),
                user_data_blind: input_user_data_blind,
            });
        }

        let mut outputs = vec![];
        for coin_attr in proposal_coinattrs.clone() {
            assert_eq!(proposal_token_id, coin_attr.token_id);
            outputs.push(coin_attr);
        }

        let dao_coin_attrs = CoinAttributes {
            public_key: dao_wallet.keypair.public,
            value: change_value,
            token_id: proposal_token_id,
            serial: pallas::Base::random(&mut OsRng),
            spend_hook: DAO_CONTRACT_ID.inner(),
            user_data: dao_bulla.inner(),
        };
        outputs.push(dao_coin_attrs.clone());

        let xfer_builder = xfer::TransferCallBuilder {
            clear_inputs: vec![],
            inputs,
            outputs,
            mint_zkbin: mint_zkbin.clone(),
            mint_pk: mint_pk.clone(),
            burn_zkbin: burn_zkbin.clone(),
            burn_pk: burn_pk.clone(),
        };

        let (xfer_params, xfer_secrets) = xfer_builder.build()?;
        let mut data = vec![MoneyFunction::TransferV1 as u8];
        xfer_params.encode(&mut data)?;
        let xfer_call = ContractCall { contract_id: *MONEY_CONTRACT_ID, data };

        // We need to extract stuff from the inputs and outputs that we'll also
        // use in the DAO::Exec call. This DAO API needs to be better.
        let mut input_value = 0;
        let mut input_value_blind = pallas::Scalar::ZERO;
        for (input, blind) in spent_coins.iter().zip(xfer_secrets.input_value_blinds.iter()) {
            input_value += input.note.value;
            input_value_blind += blind;
        }
        assert_eq!(
            pedersen_commitment_u64(input_value, input_value_blind),
            xfer_params.inputs.iter().map(|input| input.value_commit).sum()
        );

        // First output is change, second output is recipient.
        let minted_coins = xfer_secrets.minted_coins(&xfer_params);
        let user_serial = minted_coins[0].note.serial;
        let dao_serial = minted_coins[1].note.serial;

        let exec_builder = DaoExecCall {
            proposal: proposal.clone(),
            dao: dao.clone(),
            yes_vote_value,
            all_vote_value,
            yes_vote_blind,
            all_vote_blind,
            user_serial,
            dao_serial,
            input_value,
            input_value_blind,
            input_user_data_blind,
            hook_dao_exec: DAO_CONTRACT_ID.inner(),
            signature_secret: exec_signature_secret,
        };

        let (exec_params, exec_proofs) = exec_builder.make(dao_exec_zkbin, dao_exec_pk)?;
        let mut data = vec![DaoFunction::Exec as u8];
        exec_params.encode(&mut data)?;
        let exec_call = ContractCall { contract_id: *DAO_CONTRACT_ID, data };

        // Auth module
        let auth_xfer_builder = DaoAuthMoneyTransferCall {
            proposal: proposal.clone(),
            proposal_coinattrs,
            dao: dao.clone(),
            input_user_data_blind,
            dao_coin_attrs,
        };
        let (auth_xfer_params, auth_xfer_proofs) = auth_xfer_builder.make(
            dao_auth_xfer_zkbin,
            dao_auth_xfer_pk,
            dao_auth_xfer_enc_coin_zkbin,
            dao_auth_xfer_enc_coin_pk,
        )?;
        let mut data = vec![DaoFunction::AuthMoneyTransfer as u8];
        auth_xfer_params.encode(&mut data)?;
        let auth_xfer_call = ContractCall { contract_id: *DAO_CONTRACT_ID, data };

        // We need to construct this tree, where exec is the parent:
        //
        //   exec ->
        //       auth_xfer
        //       xfer
        //

        //let mut tx_builder = TransactionBuilder::new(
        //    ContractCallLeaf { call: exec_call, proofs: exec_proofs },
        //    vec![],
        //)?;
        //tx_builder
        //    .append(ContractCallLeaf { call: auth_xfer_call, proofs: auth_xfer_proofs }, vec![])?;
        //tx_builder
        //let mut tx = tx_builder.build()?;

        let mut tx = Transaction {
            calls: vec![
                DarkLeaf { data: auth_xfer_call, parent_index: Some(2), children_indexes: vec![] },
                DarkLeaf { data: xfer_call, parent_index: Some(2), children_indexes: vec![] },
                DarkLeaf { data: exec_call, parent_index: None, children_indexes: vec![0, 1] },
            ],
            proofs: vec![auth_xfer_proofs, xfer_secrets.proofs, exec_proofs],
            signatures: vec![],
        };
        let auth_xfer_sigs = vec![];
        let xfer_sigs = tx.create_sigs(&mut OsRng, &xfer_secrets.signature_secrets)?;
        let exec_sigs = tx.create_sigs(&mut OsRng, &[exec_signature_secret])?;
        tx.signatures = vec![auth_xfer_sigs, xfer_sigs, exec_sigs];
        tx_action_benchmark.creation_times.push(timer.elapsed());

        // Calculate transaction sizes
        let encoded: Vec<u8> = serialize(&tx);
        let size = std::mem::size_of_val(&*encoded);
        tx_action_benchmark.sizes.push(size);
        let base58 = bs58::encode(&encoded).into_string();
        let size = std::mem::size_of_val(&*base58);
        tx_action_benchmark.broadcasted_sizes.push(size);

        Ok((tx, xfer_params, exec_params))
    }

    pub async fn execute_dao_exec_tx(
        &mut self,
        holder: &Holder,
        tx: &Transaction,
        xfer_params: &MoneyTransferParamsV1,
        _exec_params: &DaoExecParams,
        slot: u64,
    ) -> Result<()> {
        let wallet = self.holders.get_mut(holder).unwrap();
        let tx_action_benchmark = self.tx_action_benchmarks.get_mut(&TxAction::DaoExec).unwrap();
        let timer = Instant::now();

        wallet.validator.add_transactions(&[tx.clone()], slot, true).await?;

        for output in &xfer_params.outputs {
            wallet.money_merkle_tree.append(MerkleNode::from(output.coin.inner()));
        }

        tx_action_benchmark.verify_times.push(timer.elapsed());

        Ok(())
    }
}
