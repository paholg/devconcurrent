//! DNS server: answers A/AAAA queries for known hostnames with the
//! workspace's devcontainer container IP. Unknown names get NXDOMAIN; known
//! names with no matching record (wrong family, or non-address qtype) get
//! NOERROR + empty answer.

use std::net::{IpAddr, SocketAddr};

use eyre::{Result, WrapErr};
use hickory_proto::op::{Metadata, ResponseCode};
use hickory_proto::rr::{DNSClass, Name, RData, Record, RecordType, rdata};
use hickory_server::Server;
use hickory_server::net::runtime::Time;
use hickory_server::server::{Request, RequestHandler, ResponseHandler, ResponseInfo};
use hickory_server::zone_handler::MessageResponseBuilder;
use tokio::net::{TcpListener, UdpSocket};

use crate::registry::Registry;

const TTL: u32 = 5;
const TCP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const TCP_RESPONSE_BUFFER: usize = 4096;

pub async fn serve(bind: SocketAddr, registry: Registry) -> Result<()> {
    let handler = DnsHandler { registry };
    let mut server = Server::new(handler);

    let udp = UdpSocket::bind(bind)
        .await
        .wrap_err_with(|| format!("bind dns udp on {bind}"))?;
    server.register_socket(udp);

    let tcp = TcpListener::bind(bind)
        .await
        .wrap_err_with(|| format!("bind dns tcp on {bind}"))?;
    server.register_listener(tcp, TCP_TIMEOUT, TCP_RESPONSE_BUFFER);

    tracing::info!("dns listening on {bind} (udp + tcp)");
    server
        .block_until_done()
        .await
        .map_err(|e| eyre::eyre!("dns server: {e}"))?;
    Ok(())
}

struct DnsHandler {
    registry: Registry,
}

#[async_trait::async_trait]
impl RequestHandler for DnsHandler {
    async fn handle_request<R: ResponseHandler, T: Time>(
        &self,
        request: &Request,
        mut response_handle: R,
    ) -> ResponseInfo {
        let response_code = self.respond(request, &mut response_handle).await;
        if let Some(code) = response_code {
            send_error(request, &mut response_handle, code).await
        } else {
            // Successful path already sent the response; return a stub here.
            // hickory uses the returned `ResponseInfo` for logging only.
            let metadata = Metadata::response_from_request(&request.metadata);
            ResponseInfo::from(hickory_proto::op::Header {
                metadata,
                counts: hickory_proto::op::HeaderCounts::default(),
            })
        }
    }
}

impl DnsHandler {
    /// Returns `None` on success (response already sent), or a response code if
    /// no records were emitted and the caller should reply with that code.
    async fn respond<R: ResponseHandler>(
        &self,
        request: &Request,
        response_handle: &mut R,
    ) -> Option<ResponseCode> {
        let Ok(info) = request.request_info() else {
            return Some(ResponseCode::FormErr);
        };
        let query = info.query;
        let name: Name = query.name().into();
        let qtype = query.query_type();

        let host = trim_root_dot(&name.to_ascii()).to_lowercase();
        let Some(ip) = self.registry.resolve(&host).await else {
            return Some(ResponseCode::NXDomain);
        };

        // Fall-through (non-address qtype, or family mismatch) yields an empty
        // answer with NOERROR — i.e. NODATA, not NXDOMAIN.
        let record = match (qtype, ip) {
            (RecordType::A, IpAddr::V4(v4)) => {
                let mut r = Record::from_rdata(name.clone(), TTL, RData::A(rdata::A(v4)));
                r.dns_class = DNSClass::IN;
                Some(r)
            }
            (RecordType::AAAA, IpAddr::V6(v6)) => {
                let mut r = Record::from_rdata(name.clone(), TTL, RData::AAAA(rdata::AAAA(v6)));
                r.dns_class = DNSClass::IN;
                Some(r)
            }
            _ => None,
        };

        let metadata = Metadata::response_from_request(&request.metadata);
        let builder = MessageResponseBuilder::from_message_request(request);
        let records: Vec<Record> = record.into_iter().collect();
        let response = builder.build(
            metadata,
            records.iter(),
            std::iter::empty(),
            std::iter::empty(),
            std::iter::empty(),
        );
        if let Err(e) = response_handle.send_response(response).await {
            tracing::warn!("send dns response: {e}");
        }
        None
    }
}

async fn send_error<R: ResponseHandler>(
    request: &Request,
    response_handle: &mut R,
    code: ResponseCode,
) -> ResponseInfo {
    let builder = MessageResponseBuilder::from_message_request(request);
    let response = builder.error_msg(&request.metadata, code);
    match response_handle.send_response(response).await {
        Ok(info) => info,
        Err(e) => {
            tracing::warn!("send dns error response: {e}");
            // Mirror what hickory's internal ResponseInfo::serve_failed does.
            let mut metadata = Metadata::new(
                request.metadata.id,
                hickory_proto::op::MessageType::Response,
                request.metadata.op_code,
            );
            metadata.response_code = ResponseCode::ServFail;
            ResponseInfo::from(hickory_proto::op::Header {
                metadata,
                counts: hickory_proto::op::HeaderCounts::default(),
            })
        }
    }
}

fn trim_root_dot(name: &str) -> &str {
    name.strip_suffix('.').unwrap_or(name)
}
