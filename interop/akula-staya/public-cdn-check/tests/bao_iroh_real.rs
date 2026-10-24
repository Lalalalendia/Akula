use akula_public_cdn_check::{public_cdn_bao_lab::PinnedBaoIroh,public_cdn_lab::{PublisherPinnedManifest,ChunkRequest}};
use iroh::{Endpoint,RelayMode,SecretKey,endpoint::presets,protocol::Router};
use iroh_blobs::{BlobsProtocol,Hash,store::fs::FsStore,provider::events::{EventMask,EventSender,ConnectMode,RequestMode,ObserveMode,ThrottleMode,ProviderMessage,AbortReason}};
use sha2::{Digest,Sha256};
use std::{sync::Arc,time::Duration};
fn hx(data:&[u8])->String{Sha256::digest(data).iter().map(|b|format!("{b:02x}")).collect()}
async fn ep(key:SecretKey)->Endpoint{Endpoint::builder(presets::Minimal).secret_key(key).relay_mode(RelayMode::Disabled).bind().await.unwrap()}
#[tokio::test(flavor="multi_thread")]
async fn actual_iroh_bao_public_256k_from_authorized_peer(){
 tokio::time::timeout(Duration::from_secs(90),async {
  let data=(0..512*1024).map(|i|(i%251) as u8).collect::<Vec<_>>();
  let root_hash=Hash::new(&data);
  let tmp=std::env::temp_dir().join(format!("akula-public-bao-{}-{}",std::process::id(),root_hash.to_hex()));
  assert!(!tmp.exists());std::fs::create_dir(&tmp).unwrap();
  let store=FsStore::load(&tmp).await.unwrap();
  let tag=store.add_slice(&data).await.unwrap();
  assert_eq!(tag.hash,root_hash);
  let client_key=SecretKey::generate();
  let server=ep(SecretKey::generate()).await;
  let mask=EventMask{connected:ConnectMode::Intercept,get:RequestMode::Intercept,get_many:RequestMode::Disabled,push:RequestMode::Disabled,observe:ObserveMode::Intercept,throttle:ThrottleMode::None};
  let (events,mut rx)=EventSender::channel(32,mask);
  let allowed=client_key.public();
  let gate=tokio::spawn(async move{
   while let Some(event)=rx.recv().await{
    match event{
     ProviderMessage::ClientConnected(msg)=>{
      let _=msg.tx.send(if msg.endpoint_id==Some(allowed){Ok(())}else{Err(AbortReason::Permission)}).await;
     },
     ProviderMessage::GetRequestReceived(msg)=>{
      let _=msg.tx.send(if msg.request.ranges.is_blob()&&msg.request.hash==root_hash{Ok(())}else{Err(AbortReason::Permission)}).await;
     },
     ProviderMessage::ObserveRequestReceived(msg)=>{let _=msg.tx.send(Err(AbortReason::Permission)).await;},
     _=>{}
    }
   }
  });
  let router=Router::builder(server).accept(iroh_blobs::ALPN,BlobsProtocol::new(&store,Some(events))).spawn();
  let client=ep(client_key).await;
  let conn=client.connect(router.endpoint().addr(),iroh_blobs::ALPN).await.unwrap();
  let sha=hx(&data[0..256*1024]);
  let req=ChunkRequest{content_id_sha256:&hx(&data),index:0,offset:0,length:256*1024,object_size:data.len() as u64,sha256:&sha};
  let mut reader=PinnedBaoIroh::new(&conn,*root_hash.as_bytes(),data.len() as u64,true).unwrap();
  let got=reader.read_public_part(&req).await.unwrap();
  assert_eq!(got,&data[..256*1024]);
  assert!(PinnedBaoIroh::new(&conn,*root_hash.as_bytes(),data.len() as u64,false).is_err());
  conn.close(0u8.into(),b"finished");
  router.shutdown().await.unwrap();client.close().await;gate.abort();drop(tag);drop(store);
  std::fs::remove_dir_all(&tmp).unwrap();
  println!("PUBLIC_BAO_IROH verified_256KiB=true SHA256=true BLAKE3=true private_gate=true direct_quic=true");
 }).await.unwrap();
}
