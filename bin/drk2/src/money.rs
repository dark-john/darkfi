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

use std::{collections::HashMap, process::exit};

use rand::rngs::OsRng;
use rusqlite::types::Value;

use darkfi::{zk::halo2::Field, Error, Result};
use darkfi_money_contract::{
    client::{
        MoneyNote, OwnCoin, MONEY_ALIASES_TABLE, MONEY_COINS_COL_IS_SPENT, MONEY_COINS_TABLE,
        MONEY_INFO_COL_LAST_SCANNED_SLOT, MONEY_INFO_TABLE, MONEY_KEYS_COL_IS_DEFAULT,
        MONEY_KEYS_COL_PUBLIC, MONEY_KEYS_COL_SECRET, MONEY_KEYS_TABLE, MONEY_TREE_COL_TREE,
        MONEY_TREE_TABLE,
    },
    model::Coin,
};
use darkfi_sdk::{
    bridgetree,
    crypto::{Keypair, MerkleNode, MerkleTree, Nullifier, SecretKey, TokenId},
    pasta::pallas,
};
use darkfi_serial::{deserialize, serialize};

use crate::{
    convert_named_params,
    error::{WalletDbError, WalletDbResult},
    Drk,
};

pub const BALANCE_BASE10_DECIMALS: usize = 8;

impl Drk {
    /// Initialize wallet with tables for the Money contract
    pub async fn initialize_money(&self) -> Result<()> {
        // Initialize Money wallet schema
        let wallet_schema = include_str!("../../../src/contract/money/wallet.sql");
        if let Err(e) = self.wallet.exec_batch_sql(wallet_schema).await {
            eprintln!("Error initializing Money schema: {e:?}");
            exit(2);
        }

        // Check if we have to initialize the Merkle tree.
        // We check if we find a row in the tree table, and if not, we create a
        // new tree and push it into the table.
        // For now, on success, we don't care what's returned, but in the future
        // we should actually check it.
        if self.wallet.query_single(MONEY_TREE_TABLE, &[MONEY_TREE_COL_TREE], &[]).await.is_err() {
            eprintln!("Initializing Money Merkle tree");
            let mut tree = MerkleTree::new(100);
            tree.append(MerkleNode::from(pallas::Base::ZERO));
            let _ = tree.mark().unwrap();
            self.put_money_tree(&tree).await?;
            eprintln!("Successfully initialized Merkle tree for the Money contract");
        }

        // We maintain the last scanned slot as part of the Money contract,
        // but at this moment it is also somewhat applicable to DAO scans.
        if self.last_scanned_slot().await.is_err() {
            let query = format!(
                "INSERT INTO {} ({}) VALUES (?1);",
                MONEY_INFO_TABLE, MONEY_INFO_COL_LAST_SCANNED_SLOT
            );
            if let Err(e) = self.wallet.exec_sql(&query, rusqlite::params![0]).await {
                eprintln!("Error inserting last scanned slot: {e:?}");
                exit(2);
            }
        }

        Ok(())
    }

    /// Generate a new keypair and place it into the wallet.
    pub async fn money_keygen(&self) -> Result<()> {
        eprintln!("Generating a new keypair");

        // TODO: We might want to have hierarchical deterministic key derivation.
        let keypair = Keypair::random(&mut OsRng);
        let is_default = 0;

        let query = format!(
            "INSERT INTO {} ({}, {}, {}) VALUES (?1, ?2, ?3);",
            MONEY_KEYS_TABLE,
            MONEY_KEYS_COL_IS_DEFAULT,
            MONEY_KEYS_COL_PUBLIC,
            MONEY_KEYS_COL_SECRET
        );
        if let Err(e) = self
            .wallet
            .exec_sql(
                &query,
                rusqlite::params![
                    is_default,
                    serialize(&keypair.public),
                    serialize(&keypair.secret)
                ],
            )
            .await
        {
            eprintln!("Error inserting new keypair: {e:?}");
            exit(2);
        }

        eprintln!("New address:");
        println!("{}", keypair.public);

        Ok(())
    }

    /// Fetch known unspent balances from the wallet and return them as a hashmap.
    pub async fn money_balance(&self) -> Result<HashMap<String, u64>> {
        let mut coins = self.get_coins(false).await?;
        coins.retain(|x| x.0.note.spend_hook == pallas::Base::zero());

        // Fill this map with balances
        let mut balmap: HashMap<String, u64> = HashMap::new();

        for coin in coins {
            let mut value = coin.0.note.value;

            if let Some(prev) = balmap.get(&coin.0.note.token_id.to_string()) {
                value += prev;
            }

            balmap.insert(coin.0.note.token_id.to_string(), value);
        }

        Ok(balmap)
    }

    /// Fetch all coins and their metadata related to the Money contract from the wallet.
    /// Optionally also fetch spent ones.
    /// The boolean in the returned tuple notes if the coin was marked as spent.
    pub async fn get_coins(&self, fetch_spent: bool) -> Result<Vec<(OwnCoin, bool)>> {
        let query = if fetch_spent {
            self.wallet.query_multiple(MONEY_COINS_TABLE, &[], &[]).await
        } else {
            self.wallet
                .query_multiple(
                    MONEY_COINS_TABLE,
                    &[],
                    convert_named_params! {(MONEY_COINS_COL_IS_SPENT, false)},
                )
                .await
        };

        let rows = match query {
            Ok(r) => r,
            Err(e) => {
                return Err(Error::RusqliteError(format!(
                    "[get_coins] Coins retrieval failed: {e:?}"
                )))
            }
        };

        let mut owncoins = Vec::with_capacity(rows.len());

        for row in rows {
            let Value::Blob(ref coin_bytes) = row[0] else {
                return Err(Error::ParseFailed("[get_coins] Coin bytes parsing failed"))
            };
            let coin: Coin = deserialize(coin_bytes)?;

            let Value::Integer(is_spent) = row[1] else {
                return Err(Error::ParseFailed("[get_coins] Is spent parsing failed"))
            };
            let Ok(is_spent) = u64::try_from(is_spent) else {
                return Err(Error::ParseFailed("[get_coins] Is spent parsing failed"))
            };
            let is_spent = is_spent > 0;

            let Value::Blob(ref serial_bytes) = row[2] else {
                return Err(Error::ParseFailed("[get_coins] Serial bytes parsing failed"))
            };
            let serial: pallas::Base = deserialize(serial_bytes)?;

            let Value::Blob(ref value_bytes) = row[3] else {
                return Err(Error::ParseFailed("[get_coins] Value bytes parsing failed"))
            };
            let value: u64 = deserialize(value_bytes)?;

            let Value::Blob(ref token_id_bytes) = row[4] else {
                return Err(Error::ParseFailed("[get_coins] Token ID bytes parsing failed"))
            };
            let token_id: TokenId = deserialize(token_id_bytes)?;

            let Value::Blob(ref spend_hook_bytes) = row[5] else {
                return Err(Error::ParseFailed("[get_coins] Spend hook bytes parsing failed"))
            };
            let spend_hook: pallas::Base = deserialize(spend_hook_bytes)?;

            let Value::Blob(ref user_data_bytes) = row[6] else {
                return Err(Error::ParseFailed("[get_coins] User data bytes parsing failed"))
            };
            let user_data: pallas::Base = deserialize(user_data_bytes)?;

            let Value::Blob(ref value_blind_bytes) = row[7] else {
                return Err(Error::ParseFailed("[get_coins] Value blind bytes parsing failed"))
            };
            let value_blind: pallas::Scalar = deserialize(value_blind_bytes)?;

            let Value::Blob(ref token_blind_bytes) = row[8] else {
                return Err(Error::ParseFailed("[get_coins] Token blind bytes parsing failed"))
            };
            let token_blind: pallas::Base = deserialize(token_blind_bytes)?;

            let Value::Blob(ref secret_bytes) = row[9] else {
                return Err(Error::ParseFailed("[get_coins] Secret bytes parsing failed"))
            };
            let secret: SecretKey = deserialize(secret_bytes)?;

            let Value::Blob(ref nullifier_bytes) = row[10] else {
                return Err(Error::ParseFailed("[get_coins] Nullifier bytes parsing failed"))
            };
            let nullifier: Nullifier = deserialize(nullifier_bytes)?;

            let Value::Blob(ref leaf_position_bytes) = row[11] else {
                return Err(Error::ParseFailed("[get_coins] Leaf position bytes parsing failed"))
            };
            let leaf_position: bridgetree::Position = deserialize(leaf_position_bytes)?;

            let Value::Blob(ref memo) = row[12] else {
                return Err(Error::ParseFailed("[get_coins] Memo parsing failed"))
            };

            let note = MoneyNote {
                serial,
                value,
                token_id,
                spend_hook,
                user_data,
                value_blind,
                token_blind,
                memo: memo.clone(),
            };
            let owncoin = OwnCoin { coin, note, secret, nullifier, leaf_position };

            owncoins.push((owncoin, is_spent))
        }

        Ok(owncoins)
    }

    /// Fetch all aliases from the wallet.
    /// Optionally filter using alias name and/or token id.
    pub async fn get_aliases(
        &self,
        alias_filter: Option<String>,
        token_id_filter: Option<TokenId>,
    ) -> Result<HashMap<String, TokenId>> {
        let rows = match self.wallet.query_multiple(MONEY_ALIASES_TABLE, &[], &[]).await {
            Ok(r) => r,
            Err(e) => {
                return Err(Error::RusqliteError(format!(
                    "[get_aliases] Aliases retrieval failed: {e:?}"
                )))
            }
        };

        // Fill this map with aliases
        let mut map: HashMap<String, TokenId> = HashMap::new();
        for row in rows {
            let Value::Blob(ref alias_bytes) = row[0] else {
                return Err(Error::ParseFailed("[get_aliases] Alias bytes parsing failed"))
            };
            let alias: String = deserialize(alias_bytes)?;
            if alias_filter.is_some() && alias_filter.as_ref().unwrap() != &alias {
                continue
            }

            let Value::Blob(ref token_id_bytes) = row[1] else {
                return Err(Error::ParseFailed("[get_aliases] TokenId bytes parsing failed"))
            };
            let token_id: TokenId = deserialize(token_id_bytes)?;
            if token_id_filter.is_some() && token_id_filter.as_ref().unwrap() != &token_id {
                continue
            }

            map.insert(alias, token_id);
        }

        Ok(map)
    }

    /// Fetch all aliases from the wallet, mapped by token id.
    pub async fn get_aliases_mapped_by_token(&self) -> Result<HashMap<String, String>> {
        let aliases = self.get_aliases(None, None).await?;
        let mut map: HashMap<String, String> = HashMap::new();
        for (alias, token_id) in aliases {
            let aliases_string = if let Some(prev) = map.get(&token_id.to_string()) {
                format!("{}, {}", prev, alias)
            } else {
                alias
            };

            map.insert(token_id.to_string(), aliases_string);
        }

        Ok(map)
    }

    /// Replace the Money Merkle tree in the wallet.
    pub async fn put_money_tree(&self, tree: &MerkleTree) -> Result<()> {
        // First we remove old record
        let query = format!("DELETE FROM {};", MONEY_TREE_TABLE);
        if let Err(e) = self.wallet.exec_sql(&query, &[]).await {
            eprintln!("Error removing Money tree: {e:?}");
            exit(2);
        }

        // then we insert the new one
        let query =
            format!("INSERT INTO {} ({}) VALUES (?1);", MONEY_TREE_TABLE, MONEY_TREE_COL_TREE,);
        if let Err(e) = self.wallet.exec_sql(&query, rusqlite::params![serialize(tree)]).await {
            eprintln!("Error replacing Money tree: {e:?}");
            exit(2);
        }

        Ok(())
    }

    /// Get the last scanned slot from the wallet
    pub async fn last_scanned_slot(&self) -> WalletDbResult<u64> {
        let ret = self
            .wallet
            .query_single(MONEY_INFO_TABLE, &[MONEY_INFO_COL_LAST_SCANNED_SLOT], &[])
            .await?;
        let Value::Integer(slot) = ret[0] else {
            return Err(WalletDbError::ParseColumnValueError);
        };
        let Ok(slot) = u64::try_from(slot) else {
            return Err(WalletDbError::ParseColumnValueError);
        };

        Ok(slot)
    }
}
