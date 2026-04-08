//! Minimal in-process authoritative DNS server for the lab.
//!
//! Runs on the IX bridge, serves A/AAAA/TXT records from a [`std::sync::RwLock`]-guarded
//! [`HashMap`]. Record mutations via [`DnsServer::set_host`] / [`DnsServer::set_txt`] are
//! synchronous — the record is visible to DNS queries the instant the method returns.

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

    /// Adds an A or AAAA record. Immediately visible to DNS queries.
    pub fn set_host(&self, name: &str, ip: IpAddr) -> Result<()> {
        let name = Name::from_ascii(name).context("invalid DNS name")?;
        let (rtype, rdata) = match ip {
            IpAddr::V4(v4) => (RecordType::A, RData::A(A::from(v4))),
            IpAddr::V6(v6) => (RecordType::AAAA, RData::AAAA(AAAA::from(v6))),
        };
        let record = Record::from_rdata(name.clone(), 0, rdata);
        let key = (LowerName::new(&name), rtype);
        self.records.write().unwrap().entry(key).or_default().push(record);
        Ok(())
    }

    /// Adds a TXT record. Immediately visible to DNS queries.
    pub fn set_txt(&self, name: &str, values: &[&str]) -> Result<()> {
        let name = Name::from_ascii(name).context("invalid DNS name")?;
        let txt = TXT::new(values.iter().map(|s| (*s).to_string()).collect());
        let record = Record::from_rdata(name.clone(), 0, RData::TXT(txt));
        let key = (LowerName::new(&name), RecordType::TXT);
        self.records.write().unwrap().entry(key).or_default().push(record);
        Ok(())
    }

    /// Removes all records matching the given name and type.
    pub fn remove(&self, name: &str, rtype: RecordType) -> Result<()> {
        let name = Name::from_ascii(name).context("invalid DNS name")?;
        let key = (LowerName::new(&name), rtype);
        self.records.write().unwrap().remove(&key);
        Ok(())
    }

    /// In-process lookup. Returns the first matching A or AAAA address.
    pub fn resolve(&self, name: &str) -> Option<IpAddr> {
        let name = Name::from_ascii(name).ok()?;
        let lower = LowerName::new(&name);
        let store = self.records.read().unwrap();
        // Try A first, then AAAA.
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
    let mut buf = vec![0u8; 512];
    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            result = socket.recv_from(&mut buf) => {
                let (len, src) = match result {
                    Ok(v) => v,
                    Err(e) => {
                        warn!(error = %e, "dns recv error");
                        continue;
                    }
                };
                let response_bytes = match handle_query(&records, &buf[..len]) {
                    Some(bytes) => bytes,
                    None => continue,
                };
                if let Err(e) = socket.send_to(&response_bytes, src).await {
                    warn!(error = %e, "dns send error");
                }
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

    let store = records.read().unwrap();
    let mut found = false;
    for q in query.queries() {
        let key = (q.name().into(), q.query_type());
        if let Some(recs) = store.get(&key) {
            for r in recs {
                let mut answer = r.clone();
                answer.set_dns_class(DNSClass::IN);
                response.add_answer(answer);
                found = true;
            }
        }
    }
    if !found {
        response.set_response_code(ResponseCode::NXDomain);
    }

    response.to_bytes().ok()
}
