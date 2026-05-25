//! RPC subscription session state ported from the reference `InfoSub` / `WSInfoSub`
//! surfaces.

use std::collections::{BTreeMap, BTreeSet};
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::state::role::Role;
use crate::subscriptions::subscription::{SubscriptionStream, normalize_stream_name};

static NEXT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn next_sequence() -> u64 {
    NEXT_SEQUENCE.fetch_add(1, Ordering::Relaxed) + 1
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InfoSub {
    seq: u64,
    role: Role,
    api_version: u32,
    user: String,
    forwarded_for: String,
    streams: BTreeSet<SubscriptionStream>,
}

impl InfoSub {
    pub fn new(role: Role) -> Self {
        Self {
            seq: next_sequence(),
            role,
            api_version: 0,
            user: String::new(),
            forwarded_for: String::new(),
            streams: BTreeSet::new(),
        }
    }

    pub fn with_identity(
        role: Role,
        user: impl Into<String>,
        forwarded_for: impl Into<String>,
    ) -> Self {
        Self {
            seq: next_sequence(),
            role,
            api_version: 0,
            user: user.into(),
            forwarded_for: forwarded_for.into(),
            streams: BTreeSet::new(),
        }
    }

    pub fn seq(&self) -> u64 {
        self.seq
    }

    pub fn role(&self) -> Role {
        self.role
    }

    pub fn set_role(&mut self, role: Role) {
        self.role = role;
    }

    pub fn api_version(&self) -> u32 {
        self.api_version
    }

    pub fn set_api_version(&mut self, api_version: u32) {
        self.api_version = api_version;
    }

    pub fn user(&self) -> &str {
        &self.user
    }

    pub fn set_user(&mut self, user: impl Into<String>) {
        self.user = user.into();
    }

    pub fn forwarded_for(&self) -> &str {
        &self.forwarded_for
    }

    pub fn set_forwarded_for(&mut self, forwarded_for: impl Into<String>) {
        self.forwarded_for = forwarded_for.into();
    }

    pub fn subscribe_stream(&mut self, stream: SubscriptionStream) -> bool {
        self.streams.insert(stream)
    }

    pub fn unsubscribe_stream(&mut self, stream: SubscriptionStream) -> bool {
        self.streams.remove(&stream)
    }

    pub fn unsubscribe_named_stream(&mut self, stream: &str) -> bool {
        normalize_stream_name(stream)
            .map(|stream| self.unsubscribe_stream(stream))
            .unwrap_or(false)
    }

    pub fn is_subscribed(&self, stream: SubscriptionStream) -> bool {
        self.streams.contains(&stream)
    }

    pub fn subscribed_streams(&self) -> impl Iterator<Item = SubscriptionStream> + '_ {
        self.streams.iter().copied()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WsInfoSub {
    info: InfoSub,
    remote_endpoint: SocketAddr,
    request_headers: BTreeMap<String, String>,
}

impl WsInfoSub {
    pub fn new(
        info: InfoSub,
        remote_endpoint: SocketAddr,
        request_headers: BTreeMap<String, String>,
    ) -> Self {
        Self {
            info,
            remote_endpoint,
            request_headers,
        }
    }

    pub fn from_request(
        info: InfoSub,
        remote_endpoint: SocketAddr,
        request_headers: BTreeMap<String, String>,
        api_version: u32,
        user: Option<&str>,
        forwarded_for: Option<&str>,
    ) -> Self {
        let mut info = info;
        info.set_api_version(api_version);
        if let Some(user) = user {
            info.set_user(user);
        }
        if let Some(forwarded_for) = forwarded_for {
            info.set_forwarded_for(forwarded_for);
        }
        Self::new(info, remote_endpoint, request_headers)
    }

    pub fn info(&self) -> &InfoSub {
        &self.info
    }

    pub fn info_mut(&mut self) -> &mut InfoSub {
        &mut self.info
    }

    pub fn into_info(self) -> InfoSub {
        self.info
    }

    pub fn api_version(&self) -> u32 {
        self.info.api_version()
    }

    pub fn remote_endpoint(&self) -> SocketAddr {
        self.remote_endpoint
    }

    pub fn request_headers(&self) -> &BTreeMap<String, String> {
        &self.request_headers
    }

    pub fn user(&self) -> &str {
        self.info.user()
    }

    pub fn forwarded_for(&self) -> &str {
        self.info.forwarded_for()
    }

    pub fn remote_ip(&self) -> IpAddr {
        self.remote_endpoint.ip()
    }
}
