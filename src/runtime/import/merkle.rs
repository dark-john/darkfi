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

use std::io::Cursor;

use darkfi_sdk::crypto::{MerkleNode, MerkleTree};
use darkfi_serial::{serialize, Decodable, Encodable, WriteExt};
use log::{debug, error};
use wasmer::{FunctionEnvMut, WasmPtr};

use super::acl::acl_allow;
use crate::runtime::vm_runtime::{ContractSection, Env};

/// Adds data to merkle tree. The tree, database connection, and new data to add is
/// read from `ptr` at offset specified by `len`.
/// Returns `0` on success; otherwise, returns an error-code corresponding to a
/// [`ContractError`] (defined in the SDK).
/// See also the method `merkle_add` in `sdk/src/merkle.rs`.
pub(crate) fn merkle_add(mut ctx: FunctionEnvMut<Env>, ptr: WasmPtr<u8>, len: u32) -> i64 {
    let (env, mut store) = ctx.data_and_store_mut();
    let cid = env.contract_id;

    // Enforce function ACL
    if let Err(e) = acl_allow(env, &[ContractSection::Update]) {
        error!(
            target: "runtime::merkle::merkle_add",
            "[WASM] [{}] merkle_add(): Called in unauthorized section: {}", cid, e,
        );
        return darkfi_sdk::error::CALLER_ACCESS_DENIED
    }

    // Subtract used gas. Here we count the length read from the memory slice.
    env.subtract_gas(&mut store, len as u64);

    let memory_view = env.memory_view(&store);
    let Ok(mem_slice) = ptr.slice(&memory_view, len) else {
        error!(
            target: "runtime::merkle::merkle_add",
            "[WASM] [{}] merkle_add(): Failed to make slice from ptr", cid,
        );
        return darkfi_sdk::error::INTERNAL_ERROR
    };

    let mut buf = vec![0_u8; len as usize];
    if let Err(e) = mem_slice.read_slice(&mut buf) {
        error!(
            target: "runtime::merkle::merkle_add",
            "[WASM] [{}] merkle_add(): Failed to read from memory slice: {}", cid, e,
        );
        return darkfi_sdk::error::INTERNAL_ERROR
    };

    // The buffer should deserialize into:
    // - db_info
    // - db_roots
    // - root_key (as Vec<u8>) (key being the name of the sled key in info_db where the latest root is)
    // - tree_key (as Vec<u8>) (key being the name of the sled key in info_db where the Merkle tree is)
    // - coins (as Vec<MerkleNode>) (the coins being added into the Merkle tree)
    let mut buf_reader = Cursor::new(buf);
    // FIXME: There's a type DbHandle=u32, but this should maybe be renamed
    let db_info_index: u32 = match Decodable::decode(&mut buf_reader) {
        Ok(v) => v,
        Err(e) => {
            error!(
                target: "runtime::merkle::merkle_add",
                "[WASM] [{}] merkle_add(): Failed to decode db_info DbHandle: {}", cid, e,
            );
            return darkfi_sdk::error::INTERNAL_ERROR
        }
    };
    let db_info_index = db_info_index as usize;

    let db_roots_index: u32 = match Decodable::decode(&mut buf_reader) {
        Ok(v) => v,
        Err(e) => {
            error!(
                target: "runtime::merkle::merkle_add",
                "[WASM] [{}] merkle_add(): Failed to decode db_roots DbHandle: {}", cid, e,
            );
            return darkfi_sdk::error::INTERNAL_ERROR
        }
    };
    let db_roots_index = db_roots_index as usize;

    let db_handles = env.db_handles.borrow();
    let n_dbs = db_handles.len();

    if n_dbs <= db_info_index || n_dbs <= db_roots_index {
        error!(
            target: "runtime::merkle::merkle_add",
            "[WASM] [{}] merkle_add(): Requested DbHandle that is out of bounds", cid,
        );
        return darkfi_sdk::error::INTERNAL_ERROR
    }
    let db_info = &db_handles[db_info_index];
    let db_roots = &db_handles[db_roots_index];

    // Make sure that the contract owns the dbs it wants to write to
    if db_info.contract_id != env.contract_id || db_roots.contract_id != env.contract_id {
        error!(
            target: "runtime::merkle::merkle_add",
            "[WASM] [{}] merkle_add(): Unauthorized to write to DbHandle", cid,
        );
        return darkfi_sdk::error::CALLER_ACCESS_DENIED
    }

    // This `key` represents the sled key in info where the latest root is
    let root_key: Vec<u8> = match Decodable::decode(&mut buf_reader) {
        Ok(v) => v,
        Err(e) => {
            error!(
                target: "runtime::merkle::merkle_add",
                "[WASM] [{}] merkle_add(): Failed to decode key vec: {}", cid, e,
            );
            return darkfi_sdk::error::INTERNAL_ERROR
        }
    };

    // This `key` represents the sled database tree name
    let tree_key: Vec<u8> = match Decodable::decode(&mut buf_reader) {
        Ok(v) => v,
        Err(e) => {
            error!(
                target: "runtime::merkle::merkle_add",
                "[WASM] [{}] merkle_add(): Failed to decode key vec: {}", cid, e,
            );
            return darkfi_sdk::error::INTERNAL_ERROR
        }
    };

    // This `coin` represents the leaf we're adding to the Merkle tree
    let coins: Vec<MerkleNode> = match Decodable::decode(&mut buf_reader) {
        Ok(v) => v,
        Err(e) => {
            error!(
                target: "runtime::merkle::merkle_add",
                "[WASM] [{}] merkle_add(): Failed to decode MerkleNode: {}", cid, e,
            );
            return darkfi_sdk::error::INTERNAL_ERROR
        }
    };

    // Make sure we've read the entire buffer
    if buf_reader.position() != (len as u64) {
        error!(
            target: "runtime::merkle::merkle_add",
            "[WASM] [{}] merkle_add(): Mismatch between given length, and cursor length", cid,
        );
        return darkfi_sdk::error::INTERNAL_ERROR
    }

    // Read the current tree
    let ret = match env
        .blockchain
        .lock()
        .unwrap()
        .overlay
        .lock()
        .unwrap()
        .get(&db_info.tree, &tree_key)
    {
        Ok(v) => v,
        Err(e) => {
            error!(
                target: "runtime::merkle::merkle_add",
                "[WASM] [{}] merkle_add(): Internal error getting from tree: {}", cid, e,
            );
            return darkfi_sdk::error::INTERNAL_ERROR
        }
    };

    let Some(return_data) = ret else {
        error!(
            target: "runtime::merkle::merkle_add",
            "[WASM] [{}] merkle_add(): Return data is empty", cid,
        );
        return darkfi_sdk::error::INTERNAL_ERROR
    };

    debug!(
        target: "runtime::merkle::merkle_add",
        "Serialized tree: {} bytes",
        return_data.len()
    );
    debug!(
        target: "runtime::merkle::merkle_add",
        "                 {:02x?}",
        return_data
    );

    let mut decoder = Cursor::new(&return_data);
    let set_size: u32 = match Decodable::decode(&mut decoder) {
        Ok(v) => v,
        Err(e) => {
            error!(
                target: "runtime::merkle::merkle_add",
                "[WASM] [{}] merkle_add(): Unable to read set size: {}", cid, e,
            );
            return darkfi_sdk::error::INTERNAL_ERROR
        }
    };

    let mut tree: MerkleTree = match Decodable::decode(&mut decoder) {
        Ok(v) => v,
        Err(e) => {
            error!(
                target: "runtime::merkle::merkle_add",
                "[WASM] [{}] merkle_add(): Unable to deserialize Merkle tree: {}", cid, e,
            );
            return darkfi_sdk::error::INTERNAL_ERROR
        }
    };

    // Here we add the new coins into the tree.
    let mut new_roots = vec![];

    for coin in coins {
        tree.append(coin);
        let Some(root) = tree.root(0) else {
            error!(
                target: "runtime::merkle::merkle_add",
                "[WASM] [{}] merkle_add(): Unable to read the root of tree", cid,
            );
            return darkfi_sdk::error::INTERNAL_ERROR
        };
        new_roots.push(root);
    }

    // And we serialize the tree back to bytes
    let mut tree_data = Vec::new();
    if tree_data.write_u32(set_size + new_roots.len() as u32).is_err() ||
        tree.encode(&mut tree_data).is_err()
    {
        error!(
            target: "runtime::merkle::merkle_add",
            "[WASM] [{}] merkle_add(): Couldn't reserialize modified tree", cid,
        );
        return darkfi_sdk::error::INTERNAL_ERROR
    }

    // Apply changes to overlay
    let lock = env.blockchain.lock().unwrap();
    let mut overlay = lock.overlay.lock().unwrap();
    if overlay.insert(&db_info.tree, &tree_key, &tree_data).is_err() {
        error!(
            target: "runtime::merkle::merkle_add",
            "[WASM] [{}] merkle_add(): Couldn't insert to db_info tree", cid,
        );
        return darkfi_sdk::error::INTERNAL_ERROR
    }

    // Here we add the Merkle root to our set of roots
    // TODO: We should probably make sure that this root isn't in the set
    for root in new_roots.iter() {
        // FIXME: Why were we writing the set size here?
        //let root_index: Vec<u8> = serialize(&(set_size as u32));
        //assert_eq!(root_index.len(), 4);
        debug!(
            target: "runtime::merkle::merkle_add",
            "[WASM] [{}] merkle_add(): Appending Merkle root to db: {:?}", cid, root,
        );
        let root_value: Vec<u8> = serialize(root);
        if root_value.len() != 32 {
            error!(
                target: "runtime::merkle::merkle_add",
                "[WASM] [{}] merkle_add(): Couldn't serialize root value", cid,
            );
            return darkfi_sdk::error::INTERNAL_ERROR
        }

        if overlay.insert(&db_roots.tree, &root_value, &[]).is_err() {
            error!(
                target: "runtime::merkle::merkle_add",
                "[WASM] [{}] merkle_add(): Couldn't insert to db_roots tree", cid,
            );
            return darkfi_sdk::error::INTERNAL_ERROR
        }
    }

    // Write a pointer to the latest known root
    if !new_roots.is_empty() {
        debug!(
            target: "runtime::merkle::merkle_add",
            "[WASM] [{}] merkle_add(): Replacing latest Merkle root pointer", cid,
        );

        let latest_root = serialize(new_roots.last().unwrap());
        if overlay.insert(&db_info.tree, &root_key, &latest_root).is_err() {
            error!(
                target: "runtime::merkle::merkle_add",
                "[WASM] [{}] merkle_add(): Couldn't insert latest root to db_info tree", cid,
            );
            return darkfi_sdk::error::INTERNAL_ERROR
        }
    }

    // Subtract used gas.
    // Here we count:
    // * The size of the Merkle tree we deserialized from the db.
    // * The size of the Merkle tree we serialized into the db.
    // * The size of the new Merkle roots we wrote into the db.
    drop(overlay);
    drop(lock);
    drop(db_handles);
    let spent_gas = return_data.len() + tree_data.len() + (new_roots.len() * 32);
    env.subtract_gas(&mut store, spent_gas as u64);

    darkfi_sdk::entrypoint::SUCCESS
}
