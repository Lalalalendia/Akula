use akula_public_cdn_check::public_cdn_lab::{ImmutableRangeOrigin,PublisherPinnedManifest,PublicObjectReader,OptionalChunkSource,ChunkRequest};
use akula_public_cdn_check::public_cdn_s3_lab::PinnedS3Range;
use sha2::{Digest,Sha256};
use std::{fs,io::Write};
struct Empty;
impl OptionalChunkSource for Empty {
 fn try_chunk(&mut self,_:&ChunkRequest<'_>)->Result<Option<Vec<u8>>,()>{Ok(None)}
}
#[test]
fn real_minio_64m_ranges_and_version_fence(){
 let file=fs::read(std::env::var("AKULA_TEST_MANIFEST").expect("fixture manifest")).unwrap();
 let pin=std::env::var("AKULA_TEST_MANIFEST_SHA256").unwrap();
 let manifest=PublisherPinnedManifest::from_pinned_json(&file,&pin).unwrap();
 assert_eq!(manifest.object_size(),64*1024*1024);
 assert_eq!(manifest.chunk_count(),256);
 let url=manifest.origin_url_for_allowlisted_adapter().to_string();
 let mut origin=PinnedS3Range::for_test_loopback_minio(&manifest,&url).unwrap();
 let mut cache=Empty;let mut peer=Empty;
 let mut r=PublicObjectReader::new(&manifest,&mut cache,&mut peer,&mut origin);
 let mut sink=ShaSink::default();
 let receipt=r.stream_into(&mut sink).unwrap();
 assert_eq!(receipt.bytes,64*1024*1024);
 assert_eq!(sink.sha.finalize().as_slice(),receipt.object_sha256);
 assert_eq!(sink.blake.finalize().as_bytes(),&receipt.staya_content_id.0);
 assert_eq!(receipt.metrics.verified_origin_bytes,receipt.bytes);
 assert_eq!(receipt.metrics.origin_requests,256);
 let mut stale=PinnedS3Range::new_https(&manifest,&url,Some("\"wrong\""));
 assert!(stale.is_err(),"test-only http must be unavailable on HTTPS constructor");
 println!("REAL_MINIO 64MiB origin_gets={} sha256={} blake3={} bounded_write_calls={}",
  receipt.metrics.origin_requests,manifest.content_id_sha256(),receipt.staya_content_id.0.iter().map(|b|format!("{b:02x}")).collect::<String>(),sink.calls);
}
struct ShaSink{sha:Sha256,blake:blake3::Hasher,calls:usize}
impl Default for ShaSink{fn default()->Self{Self{sha:Sha256::new(),blake:blake3::Hasher::new(),calls:0}}}
impl Write for ShaSink{
 fn write(&mut self,x:&[u8])->std::io::Result<usize>{assert!(x.len()<=256*1024);self.sha.update(x);self.blake.update(x);self.calls+=1;Ok(x.len())}
 fn flush(&mut self)->std::io::Result<()>{Ok(())}
}
