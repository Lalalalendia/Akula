//! VIDEO-18c-02b-04: read native Windows RT_VERSION from the actual PE.
 //! Non-release, read-only exact-PR-head witness; never an updater.
#[cfg(not(windows))]
fn main() {
    eprintln!("VIDEO18C02B04_HOLD: Windows hosted PR only");
    std::process::exit(2);
}
#[cfg(windows)]
mod win {
    use std::env;
    use std::ffi::{OsStr, c_void};
    use std::fs::{self, File};
    use std::io::Read;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::ptr::{null, null_mut};
    use std::process::Command;

    #[link(name = "version")]
    unsafe extern "system" {
        fn GetFileVersionInfoSizeW(file: *const u16, handle: *mut u32) -> u32;
        fn GetFileVersionInfoW(file: *const u16, handle: u32, len: u32, data: *mut c_void) -> i32;
        fn VerQueryValueW(
            data: *const c_void, key: *const u16,
            result: *mut *mut c_void, len: *mut u32,
        ) -> i32;
    }
    const OLD: &str = "0.3.20";
    const PHANTOM: &str = "0.3.21";
    fn hold(reason: &str) -> String { format!("VIDEO18C02B04_HOLD: {reason}") }

    fn triple(v: &str) -> Result<[u16; 3], String> {
        let parts: Vec<_> = v.split('.').collect();
        if parts.len() != 3 { return Err(hold("requires canonical major.minor.patch")); }
        let mut out = [0u16;3];
        for (i,p) in parts.into_iter().enumerate() {
            if p.is_empty() || (p.len()>1 && p.starts_with('0'))
                || !p.bytes().all(|x|x.is_ascii_digit()) {
                return Err(hold("noncanonical version"));
            }
            out[i]=p.parse().map_err(|_| hold("version overflow"))?;
        }
        Ok(out)
    }
    fn utf16(raw: &OsStr) -> Vec<u16> {
        raw.encode_wide().chain(std::iter::once(0)).collect()
    }
    fn native_version(path: &Path) -> Result<[u16; 3], String> {
        let attr=fs::symlink_metadata(path).map_err(|e|e.to_string())?;
        if !attr.is_file() || attr.file_type().is_symlink() || attr.len()<512 {
            return Err(hold("candidate is missing/not a real PE"));
        }
        let mut file=File::open(path).map_err(|e|e.to_string())?;
        let mut mz=[0u8;2];
        file.read_exact(&mut mz).map_err(|e|e.to_string())?;
        if &mz != b"MZ" {return Err(hold("not MZ executable"));}

        let wide=utf16(path.as_os_str());
        let mut h=0u32;
        let size=unsafe { GetFileVersionInfoSizeW(wide.as_ptr(), &mut h) };
        // A minimal VS_VERSIONINFO is 6-byte header, UTF-16 key/padding
        // (34 bytes), and exactly 52 bytes of VS_FIXEDFILEINFO = 92 bytes.
        if !(92..=256*1024).contains(&size) {
            return Err(hold("missing/oversized Windows RT_VERSION; never infer from filename"));
        }
        let mut bytes=vec![0u8;size as usize];
        if unsafe { GetFileVersionInfoW(wide.as_ptr(),0,size,bytes.as_mut_ptr().cast()) }==0 {
            return Err(hold("failed reading native VERSIONINFO"));
        }
        let root=[b'\\' as u16,0];
        let mut ptr:*mut c_void=null_mut();
        let mut value_len=0u32;
        if unsafe {VerQueryValueW(bytes.as_ptr().cast(),root.as_ptr(),&mut ptr,&mut value_len)}==0
            || ptr.is_null() || value_len!=52 {
            return Err(hold("no exact VS_FIXEDFILEINFO root"));
        }
        let fixed=unsafe {std::ptr::read_unaligned(ptr.cast::<[u32;13]>())};
        if fixed[0]!=0xFEEF04BD || fixed[1]!=0x0001_0000 || fixed[10]!=0
            || fixed[8]!=0x0004_0004 || fixed[9]!=1
            || fixed[2]!=fixed[4] || fixed[3]!=fixed[5]
            || (fixed[3] & 0xffff)!=0 {
            return Err(hold("invalid native FILEVERSION/PRODUCTVERSION fixed resource"));
        }
        Ok([(fixed[2]>>16) as u16,(fixed[2] & 0xffff) as u16,
            (fixed[3]>>16) as u16])
    }
    fn guard() -> Result<(),String> {
        for (key,value) in [
            ("GITHUB_ACTIONS","true"), ("GITHUB_EVENT_NAME","pull_request"),
            ("RUNNER_ENVIRONMENT","github-hosted"), ("RUNNER_OS","Windows"),
        ] {
            if env::var(key).as_deref()!=Ok(value) {
                return Err(hold(&format!("not hosted Windows PR: {key}")));
            }
        }
        let root=env::var("GITHUB_WORKSPACE").map_err(|_|hold("no workspace"))?;
        let head=env::var("EXPECTED_HEAD_SHA").map_err(|_|hold("no expected HEAD"))?;
        if head.len()!=40 || !head.bytes().all(|b|b.is_ascii_hexdigit()) {
            return Err(hold("invalid HEAD SHA"));
        }
        let output=Command::new("git").arg("-C").arg(&root).args(["rev-parse","HEAD"])
            .output().map_err(|e|e.to_string())?;
        if !output.status.success() || String::from_utf8_lossy(&output.stdout).trim()!=head {
            return Err(hold("checkout differs from exact PR head"));
        }
        Ok(())
    }
    pub fn run()->Result<(),String> {
        guard()?;
        let args:Vec<String>=env::args().collect();
        if !(args.len()==4 && args[1]=="--identify"
            || args.len()==6 && args[1]=="--admit") {
            return Err(hold("usage: --identify APP SOURCE or --admit APP SOURCE INSTALLED SIGNED"));
        }
        let root=fs::canonicalize(env::var("GITHUB_WORKSPACE").map_err(|e|e.to_string())?)
            .map_err(|e|e.to_string())?;
        let app=fs::canonicalize(&args[2]).map_err(|e|e.to_string())?;
        let expected=fs::canonicalize(root.join(".artifacts/cargo-native/release/aquilum-native.exe"))
            .map_err(|e|e.to_string())?;
        if app!=expected {return Err(hold("only exact source-built native EXE admitted"));}
        let embedded=native_version(&app)?;
        let source=triple(&args[3])?;
        if embedded!=source {return Err(hold("native PE version differs from Cargo source version"));}
        if args[1]=="--identify" {
            println!("VIDEO18C02B04_NATIVE_PE_IDENTITY_PASS version={} cargo_source_match=1 release=0",
                args[3]);
            return Ok(());
        }
        let installed=triple(&args[4])?;
        let signed=triple(&args[5])?;
        if signed!=embedded || embedded<=installed {
            return Err(hold("signed label is not a genuine newer native PE version"));
        }
        println!("VIDEO18C02B04_STRICT_VERSION_FLOOR_PASS source=1 pe=1 signed=1 release=0");
        Ok(())
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test] fn canonical_versions() {
            assert_eq!(triple(OLD).unwrap(),[0,3,20]);
            assert_eq!(triple(PHANTOM).unwrap(),[0,3,21]);
            for x in ["0.3.020","v0.3.21","0.3.21-dev","0.3","0.3.65536","0.3.21.0"] {
                assert!(triple(x).is_err(),"{x}");
            }
        }
        #[test] fn phantom_label_cannot_prove_new_native_build() {
            let executable=triple(OLD).unwrap();
            let advertised=triple(PHANTOM).unwrap();
            assert_ne!(executable,advertised);
            assert!(executable<=triple(OLD).unwrap());
        }
    }
}
#[cfg(windows)]
fn main() {
    if let Err(e)=win::run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
