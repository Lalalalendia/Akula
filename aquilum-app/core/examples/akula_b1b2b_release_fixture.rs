//! B1b2b CI-only release wire bridge to the existing B1a Rust verifier.
//! Test signing happens on the runner. Only an independently provisioned
//! public key travels to the disposable Ubuntu guest. Not a release authority.
use aquilum_core::server_release_lab::{
    verify_signed_release, ReleaseFloor, ReleaseManifestV1, SignedRelease,
    TargetPlatform, TrustedReleaseSigner, Version, MAX_NODE_PACKAGE_BYTES,
};
use ring::{digest, signature::{Ed25519KeyPair, KeyPair}};
use std::fs::{self, File};
use std::io::Read;
use std::time::{SystemTime, UNIX_EPOCH};

const DOMAIN: &[u8] = b"Akula Node B1a signed release manifest v1\0";
const SIGNER: &str = "akula-b1b2b-ephemeral-test";
const MAX_MANIFEST: usize = 256;

fn take<const N: usize>(buf: &[u8], at: &mut usize) -> Result<[u8; N], String> {
    let end=at.checked_add(N).ok_or("offset overflow")?;
    let slice=buf.get(*at..end).ok_or("truncated canonical manifest")?;
    *at=end;
    slice.try_into().map_err(|_|"invalid manifest field".into())
}
fn u32be(buf: &[u8], at: &mut usize) -> Result<u32, String> {
    Ok(u32::from_be_bytes(take::<4>(buf,at)?))
}
fn u64be(buf: &[u8], at: &mut usize) -> Result<u64, String> {
    Ok(u64::from_be_bytes(take::<8>(buf,at)?))
}
fn from_wire(buf: &[u8]) -> Result<ReleaseManifestV1, String> {
    if buf.len()>MAX_MANIFEST || !buf.starts_with(DOMAIN) {
        return Err("invalid B1a signing domain or oversized manifest".into());
    }
    let mut at=DOMAIN.len();
    if take::<1>(buf,&mut at)? != [1] || take::<1>(buf,&mut at)? != [1] {
        return Err("unsupported B1a schema or platform".into());
    }
    let version=Version {
        major:u32be(buf,&mut at)?,minor:u32be(buf,&mut at)?,patch:u32be(buf,&mut at)?,
    };
    let release_sequence=u64be(buf,&mut at)?;
    let package_size=u64be(buf,&mut at)?;
    let not_before_unix=u64be(buf,&mut at)?;
    let expires_unix=u64be(buf,&mut at)?;
    let package_sha256=take::<32>(buf,&mut at)?;
    let len=take::<1>(buf,&mut at)?[0] as usize;
    let end=at.checked_add(len).ok_or("signer length overflow")?;
    let signer=buf.get(at..end).ok_or("truncated signer")?;
    at=end;
    let signer_id=String::from_utf8(signer.to_vec()).map_err(|_|"invalid UTF-8 signer")?;
    let m=ReleaseManifestV1 {
        platform:TargetPlatform::LinuxX86_64,version,release_sequence,package_size,
        package_sha256,not_before_unix,expires_unix,signer_id,
    };
    if at!=buf.len() || m.canonical_signed_bytes()!=buf {
        return Err("noncanonical B1a wire manifest".into());
    }
    Ok(m)
}
fn now()->Result<u64,String>{
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)
        .map_err(|e|e.to_string())?.as_secs())
}
fn exact<const N:usize>(file:&str)->Result<[u8;N],String>{
    let data=fs::read(file).map_err(|e|format!("{file}: {e}"))?;
    let len=data.len();
    data.try_into().map_err(|_|format!("{file}: expected {N} bytes, found {len}"))
}
fn package_sha(path:&str)->Result<([u8;32],u64),String>{
    let mut file=File::open(path).map_err(|e|e.to_string())?;
    let size=file.metadata().map_err(|e|e.to_string())?.len();
    if size==0 || size>MAX_NODE_PACKAGE_BYTES {return Err("invalid package size".into());}
    let mut sha=digest::Context::new(&digest::SHA256);
    let mut seen=0u64;
    let mut buf=[0u8;32768];
    loop{
        let n=file.read(&mut buf).map_err(|e|e.to_string())?;
        if n==0 {break;}
        seen=seen.checked_add(n as u64).ok_or("overflow")?;
        if seen>size {return Err("package grew while signing".into());}
        sha.update(&buf[..n]);
    }
    if seen!=size {return Err("package changed while signing".into());}
    Ok((sha.finish().as_ref().try_into().map_err(|_|"SHA-256 length")?,size))
}
fn make(args:&[String])->Result<(),String>{
    if args.len()!=10 {return Err("make PACKAGE META SIG PUBLIC SEED SEQ MAJ MIN PATCH".into());}
    let sequence:u64=args[6].parse().map_err(|_|"invalid sequence")?;
    let major:u32=args[7].parse().map_err(|_|"invalid major")?;
    let minor:u32=args[8].parse().map_err(|_|"invalid minor")?;
    let patch:u32=args[9].parse().map_err(|_|"invalid patch")?;
    let key=Ed25519KeyPair::from_seed_unchecked(&exact::<32>(&args[5])?)
        .map_err(|_|"invalid ephemeral signing seed")?;
    let (sha,size)=package_sha(&args[1])?;
    let timestamp=now()?;
    let m=ReleaseManifestV1{
        platform:TargetPlatform::LinuxX86_64,
        version:Version {major,minor,patch},release_sequence:sequence,
        package_size:size,package_sha256:sha,
        not_before_unix:timestamp.saturating_sub(120),
        expires_unix:timestamp.checked_add(7200).ok_or("invalid timestamp")?,
        signer_id:SIGNER.into(),
    };
    let signed=m.canonical_signed_bytes();
    let sig=key.sign(&signed);
    fs::write(&args[2],signed).map_err(|e|e.to_string())?;
    fs::write(&args[3],sig.as_ref()).map_err(|e|e.to_string())?;
    fs::write(&args[4],key.public_key().as_ref()).map_err(|e|e.to_string())?;
    println!("B1B2B_EPHEMERAL_SIGNED seq={sequence} bytes={size}");
    Ok(())
}
fn verify(args:&[String])->Result<(),String>{
    if args.len()!=5 {return Err("verify PACKAGE META SIG TRUSTED_PUBLIC_KEY".into());}
    let wire=fs::read(&args[2]).map_err(|e|e.to_string())?;
    let m=from_wire(&wire)?;
    let signer=TrustedReleaseSigner{
        signer_id:SIGNER.into(),ed25519_public:exact::<32>(&args[4])?,
    };
    let signed=SignedRelease{manifest:m,signature:exact::<64>(&args[3])?};
    let verified=verify_signed_release(&signed,&signer,ReleaseFloor::initial(),now()?)
        .map_err(|e|format!("B1a metadata signature/freshness: {e:?}"))?;
    let mut file=File::open(&args[1]).map_err(|e|e.to_string())?;
    let receipt=verified.verify_package_stream(&mut file)
        .map_err(|e|format!("B1a exact package stream: {e:?}"))?;
    println!("B1B2B_B1A_VERIFIED sequence={} bytes={}",
        receipt.proposed_floor.sequence,receipt.byte_length);
    Ok(())
}
fn main(){
    let args:Vec<String>=std::env::args().skip(1).collect();
    let outcome=match args.first().map(String::as_str){
        Some("make")=>make(&args),
        Some("verify")=>verify(&args),
        _=>Err("usage: make|verify (ephemeral CI lab only)".into()),
    };
    if let Err(e)=outcome {
        eprintln!("B1B2B_REFUSED: {e}");
        std::process::exit(2);
    }
}
