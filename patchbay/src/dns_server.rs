//! Minimal in-process authoritative DNS server for the lab.
//!
//! Runs on the IX bridge, serves A/AAAA/TXT records from a [`std::sync::RwLock`]-guarded
//! [`HashMap`]. Record mutations via [`DnsServer::set_host`] / [`DnsServer::set_txt`] are
//! synchronous — the record is visible to DNS queries the instant the method returns.
//!
//! ## Limitations
//!
//! - **No TCP fallback.** Only UDP is supported. Responses exceeding 512 bytes
//!   will be truncated. This is fine for lab use with short names.
//! - **FQDN trailing dots.** DNS names passed to [`DnsServer::set_host`] and
//!   [`DnsServer::set_txt`] are parsed as DNS wire names. Use a trailing dot
//!   for fully-qualified names (`"relay.test."`). Device-level
//!   [`set_host`](crate::Device::set_host) writes to `/etc/hosts` where trailing
//!   dots are not used — these are different namespaces.
//! - **TXT records are not queryable via [`DnsServer::resolve`]** — it only
//!   returns A/AAAA addresses. Query TXT records via DNS (e.g. `dig`).

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, RwLock};

use anyhow::{Context, Result};
use hickory_proto::op::{Message, MessageType, ResponseCode};
use hickory_proto::rr::rdata::{A, AAAA, TXT};
use hickory_proto::rr::{DNSClass, LowerName, Name, RData, Record, RecordType};
use hickory_proto::serialize::binary::{BinDecodable, BinEncodable};
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::netns;

/// Default TTL for DNS records (seconds). Short enough for quick updates,
/// long enough to avoid per-query overhead in resolvers.
const DEFAULT_TTL: u32 = 1;

type RecordStore = HashMap<(LowerName, RecordType), Vec<Record>>;

/// In-process DNS server running on the IX bridge.
///
/// Records are stored in a [`HashMap`] behind a [`std::sync::RwLock`].
/// All mutation methods are synchronous and guarantee the record is queryable
/// via DNS before returning.
pub struct DnsServer {
    records: Arc<RwLock<RecordStore>>,
    shutdown: CancellationToken,
}

impl DnsServer {
    /// Starts the DNS server on the IX bridge inside the root namespace.
    pub(crate) async fn start(
        netns: &Arc<netns::NetnsManager>,
        root_ns: &str,
        ix_gw: Ipv4Addr,
    ) -> Result<Self> {
        let records: Arc<RwLock<RecordStore>> = Arc::new(RwLock::new(HashMap::new()));
        let shutdown = CancellationToken::new();

        let bind_addr = SocketAddr::new(IpAddr::V4(ix_gw), 53);
        let (tx, rx) = tokio::sync::oneshot::channel();

        let rt = netns.rt_handle_for(root_ns)?;
        let records2 = records.clone();
        let cancel = shutdown.clone();
        rt.spawn(async move {
            match tokio::net::UdpSocket::bind(bind_addr).await {
                Ok(socket) => {
                    let _ = tx.send(Ok(()));
                    run(records2, socket, cancel).await;
                }
                Err(e) => {
                    let _ = tx.send(Err(e));
                }
            }
        });

        rx.await
            .map_err(|_| anyhow::anyhow!("dns server task exited before bind"))?
            .context("dns server bind failed")?;

        debug!(addr = %bind_addr, "dns server started");
        Ok(Self { records, shutdown })
    }

    /// Sets an A or AAAA record, replacing any previous record of the same type
    /// for this name. Immediately visible to DNS queries.
    pub fn set_host(&self, name: &str, ip: IpAddr) -> Result<()> {
        let name = Name::from_ascii(name).context("invalid DNS name")?;
        let (rtype, rdata) = match ip {
            IpAddr::V4(v4) => (RecordType::A, RData::A(A::from(v4))),
            IpAddr::V6(v6) => (RecordType::AAAA, RData::AAAA(AAAA::from(v6))),
        };
        let record = Record::from_rdata(name.clone(), DEFAULT_TTL, rdata);
        let key = (LowerName::new(&name), rtype);
        let mut store = self.records.write().expect("poisoned");
        store.insert(key, vec![record]);
        Ok(())
    }

    /// Sets a TXT record, replacing any previous TXT record for this name.
    /// Immediately visible to DNS queries.
    pub fn set_txt(&self, name: &str, values: &[&str]) -> Result<()> {
        let name = Name::from_ascii(name).context("invalid DNS name")?;
        let txt = TXT::new(values.iter().map(|s| s.to_string()).collect());
        let record = Record::from_rdata(name.clone(), DEFAULT_TTL, RData::TXT(txt));
        let key = (LowerName::new(&name), RecordType::TXT);
        let mut store = self.records.write().expect("poisoned");
        store.insert(key, vec![record]);
        Ok(())
    }

    /// Removes all records matching the given name and type.
    pub fn remove(&self, name: &str, rtype: RecordType) -> Result<()> {
        let name = Name::from_ascii(name).context("invalid DNS name")?;
        let key = (LowerName::new(&name), rtype);
        self.records.write().expect("poisoned").remove(&key);
        Ok(())
    }

    /// In-process lookup. Returns the first matching A or AAAA address.
    pub fn resolve(&self, name: &str) -> Option<IpAddr> {
        let name = Name::from_ascii(name).ok()?;
        let lower = LowerName::new(&name);
        let store = self.records.read().expect("poisoned");
        for rtype in [RecordType::A, RecordType::AAAA] {
            if let Some(recs) = store.get(&(lower.clone(), rtype)) {
                for r in recs {
                    match r.data() {
                        RData::A(a) => return Some(IpAddr::V4((*a).into())),
                        RData::AAAA(aaaa) => return Some(IpAddr::V6((*aaaa).into())),
                        _ => {}
                    }
                }
            }
        }
        None
    }
}

impl Drop for DnsServer {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

// ── UDP server loop ──────────────────────────────────────────────────

async fn run(records: Arc<RwLock<RecordStore>>, socket: tokio::net::UdpSocket, cancel: CancellationToken) {
    cancel.run_until_cancelled(serve_loop(records, socket)).await;
}

async fn serve_loop(records: Arc<RwLock<RecordStore>>, socket: tokio::net::UdpSocket) {
    let mut buf = vec![0u8; 4096];
    loop {
        let (len, src) = match socket.recv_from(&mut buf).await {
            Ok(v) => v,
            Err(e) => {
                warn!(error = %e, "dns recv error");
                continue;
            }
        };
        if let Some(response_bytes) = handle_query(&records, &buf[..len]) {
            if let Err(e) = socket.send_to(&response_bytes, src).await {
                warn!(error = %e, "dns send error");
            }
        }
    }
}

fn handle_query(records: &RwLock<RecordStore>, buf: &[u8]) -> Option<Vec<u8>> {
    let query = Message::from_bytes(buf).ok()?;
    if query.message_type() != MessageType::Query {
        return None;
    }

    let mut response = Message::new();
    response.set_id(query.id());
    response.set_message_type(MessageType::Response);
    response.set_op_code(query.op_code());
    response.set_recursion_desired(query.recursion_desired());
    response.set_recursion_available(false);
    response.set_authoritative(true);
    response.add_queries(query.queries().iter().cloned());

    let store = records.read().expect("poisoned");
    let mut found_answers = false;
    let mut name_exists = false;
    for q in query.queries() {
        let qname: LowerName = q.name().into();
        // Check if any record type exists for this name.
        if !name_exists {
            name_exists = store.keys().any(|(n, _)| *n == qname);
        }
        let key = (qname, q.query_type());
        if let Some(recs) = store.get(&key) {
            for r in recs {
                let mut answer = r.clone();
                answer.set_dns_class(DNSClass::IN);
                response.add_answer(answer);
                found_answers = true;
            }
        }
    }
    if !found_answers {
        if name_exists {
            // Name exists but not for the queried type → NOERROR with empty answer.
            response.set_response_code(ResponseCode::NoError);
        } else {
            response.set_response_code(ResponseCode::NXDomain);
        }
    }

    response.to_bytes().ok()
}
