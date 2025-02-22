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

use std::{sync::Arc, time::UNIX_EPOCH};

use async_trait::async_trait;
use log::debug;
use smol::Executor;

use super::{
    super::{
        channel::ChannelPtr,
        hosts::store::HostsPtr,
        message::{AddrsMessage, GetAddrsMessage},
        message_subscriber::MessageSubscription,
        p2p::P2pPtr,
        settings::SettingsPtr,
    },
    protocol_base::{ProtocolBase, ProtocolBasePtr},
};
use crate::{net::hosts::refinery::ping_node, Result};

/// Implements the seed protocol
pub struct ProtocolSeed {
    channel: ChannelPtr,
    hosts: HostsPtr,
    settings: SettingsPtr,
    addr_sub: MessageSubscription<AddrsMessage>,
    p2p: P2pPtr,
}

const PROTO_NAME: &str = "ProtocolSeed";

impl ProtocolSeed {
    /// Create a new seed protocol.
    pub async fn init(channel: ChannelPtr, p2p: P2pPtr) -> ProtocolBasePtr {
        let hosts = p2p.hosts();
        let settings = p2p.settings();

        // Create a subscription to address message
        let addr_sub =
            channel.subscribe_msg::<AddrsMessage>().await.expect("Missing addr dispatcher!");

        Arc::new(Self { channel, hosts, settings, addr_sub, p2p })
    }

    /// Sends own external addresses over a channel. Imports own external addresses
    /// from settings, then adds those addresses to an addrs message and sends it
    /// out over the channel.
    pub async fn send_my_addrs(&self) -> Result<()> {
        debug!(target: "net::protocol_seed::send_my_addrs()", "[START]");
        // Do nothing if external addresses are not configured
        if self.settings.external_addrs.is_empty() {
            debug!(target: "net::protocol_seed::send_my_addrs()",
            "External address is not configured. Stopping");
            return Ok(())
        }

        // Do nothing if advertise is set to false
        if !self.settings.advertise {
            debug!(target: "net::protocol_seed::send_my_addrs()",
            "Advertise is set to false. Stopping");
            return Ok(())
        }

        let mut addrs = vec![];
        for addr in self.settings.external_addrs.clone() {
            debug!(target: "net::protocol_seed::send_my_addrs()", "Attempting to ping self");

            // See if we can do a version exchange with ourself.
            if ping_node(&addr, self.p2p.clone()).await {
                // We're online. Update last_seen and broadcast our address.
                let last_seen = UNIX_EPOCH.elapsed().unwrap().as_secs();
                addrs.push((addr, last_seen));
            } else {
                debug!(target: "net::protocol_seed::send_my_addrs()", "Ping self failed");
                return Ok(())
            }
        }
        debug!(target: "net::protocol_seed::send_my_addrs()", "Broadcasting address");
        let ext_addr_msg = AddrsMessage { addrs };
        self.channel.send(&ext_addr_msg).await?;
        debug!(target: "net::protocol_seed::send_my_addrs()", "[END]");

        Ok(())
    }
}

#[async_trait]
impl ProtocolBase for ProtocolSeed {
    /// Starts the seed protocol. Creates a subscription to the address message,
    /// then sends our address to the seed server. Sends a get-address message
    /// and receives an address messsage.
    async fn start(self: Arc<Self>, _ex: Arc<Executor<'_>>) -> Result<()> {
        debug!(target: "net::protocol_seed::start()", "START => address={}", self.channel.address());

        // Send own address to the seed server
        self.send_my_addrs().await?;

        // Send get address message
        let get_addr = GetAddrsMessage {
            max: self.settings.outbound_connections as u32,
            transports: self.settings.allowed_transports.clone(),
        };
        self.channel.send(&get_addr).await?;

        // Receive addresses
        let addrs_msg = self.addr_sub.receive().await?;
        debug!(
            target: "net::protocol_seed::start()",
            "Received {} addrs from {}", addrs_msg.addrs.len(), self.channel.address(),
        );
        debug!(
            target: "net::protocol_seed::start()",
            "Appending to greylist...",
        );
        self.hosts.greylist_store_or_update(&addrs_msg.addrs).await;

        debug!(target: "net::protocol_seed::start()", "END => address={}", self.channel.address());
        Ok(())
    }

    fn name(&self) -> &'static str {
        PROTO_NAME
    }
}
