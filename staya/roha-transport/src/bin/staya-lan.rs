//! Standalone, manually addressed LAN proof-of-concept. No files or Sync Core.
use akula_roha_transport::{AllowedPeers, StayaTransport};
use iroh::{EndpointAddr, EndpointId, SecretKey};
use std::{env, net::SocketAddr, str::FromStr, time::Duration};

fn decode_key(s: &str) -> Result<SecretKey, String> {
    if s.len() != 64 { return Err("secret key must be 64 hex characters".into()); }
    let mut raw = [0u8; 32];
    for (i, byte) in raw.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&s[2*i..2*i+2], 16).map_err(|_| "invalid secret hex")?;
    }
    Ok(SecretKey::from_bytes(&raw))
}

fn parse_id(s: &str) -> Result<EndpointId, String> {
    EndpointId::from_str(s).map_err(|e| format!("invalid peer id: {e}"))
}

fn usage() -> &'static str {
    "usage:\n  staya-lan gen-key\n  staya-lan serve SECRET_HEX ALLOWED_PEER_ID BIND_IP:UDP_PORT\n  staya-lan ping SECRET_HEX SERVER_PEER_ID SERVER_IP:UDP_PORT\nWARNING: SECRET_HEX is sensitive. Do not paste into reports, logs or shared terminals.\n"
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), String> {
    let args: Vec<String> = env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("gen-key") if args.len() == 2 => {
            let key = SecretKey::generate();
            let hex: String = key.to_bytes().iter().map(|b| format!("{b:02x}")).collect();
            println!("endpoint_id={}", key.public());
            println!("secret_hex={hex}");
            eprintln!("Keep secret_hex private. This prototype has no OS key storage.");
        }
        Some("serve") if args.len() == 5 => {
            let key = decode_key(&args[2])?;
            let allowed = parse_id(&args[3])?;
            let socket: SocketAddr = args[4].parse().map_err(|_| "invalid bind socket")?;
            let transport = StayaTransport::bind_on(key, AllowedPeers::new([allowed]), socket).await?;
            println!("endpoint_id={}", transport.id());
            println!("local_udp_sockets={:?}", transport.bound_sockets());
            println!("relay=disabled discovery=disabled allowed_peer={allowed}");
            loop {
                let conn = transport.accept_one().await?;
                let (mut send, mut recv) = tokio::time::timeout(
                    Duration::from_secs(15), conn.accept_bi()
                ).await.map_err(|_| "stream accept timeout")?.map_err(|e| e.to_string())?;
                let msg = tokio::time::timeout(Duration::from_secs(15), recv.read_to_end(64))
                    .await.map_err(|_| "receive timeout")?.map_err(|e| e.to_string())?;
                if msg.as_slice() != b"akula-ping-v0" {
                    conn.close(1u8.into(), b"unexpected payload");
                    continue;
                }
                send.write_all(b"akula-pong-v0").await.map_err(|e| e.to_string())?;
                send.finish().map_err(|e| e.to_string())?;
                println!("ping_ok remote={}", conn.remote_id());
            }
        }
        Some("ping") if args.len() == 5 => {
            let key = decode_key(&args[2])?;
            let server = parse_id(&args[3])?;
            let socket: SocketAddr = args[4].parse().map_err(|_| "invalid peer socket")?;
            let local = StayaTransport::bind(key, AllowedPeers::new([server])).await?;
            let addr = EndpointAddr::new(server).with_ip_addr(socket);
            let conn = local.connect(addr).await?;
            let (mut send, mut recv) = conn.open_bi().await.map_err(|e| e.to_string())?;
            send.write_all(b"akula-ping-v0").await.map_err(|e| e.to_string())?;
            send.finish().map_err(|e| e.to_string())?;
            let received = tokio::time::timeout(
                Duration::from_secs(15), recv.read_to_end(64)
            ).await.map_err(|_| "reply timeout")?.map_err(|e| e.to_string())?;
            if received.as_slice() != b"akula-pong-v0" {
                return Err("wrong response".into());
            }
            println!("direct_lan_ping_ok peer={server} relay=disabled");
            local.close().await;
        }
        _ => return Err(usage().into()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn secret_hex_roundtrip_and_bad_keys() {
        let k = SecretKey::generate();
        let hex: String = k.to_bytes().iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(decode_key(&hex).unwrap().public(), k.public());
        assert!(decode_key("bad").is_err());
        assert!(decode_key(&"gg".repeat(32)).is_err());
    }
}
