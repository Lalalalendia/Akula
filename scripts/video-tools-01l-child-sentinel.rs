//! A separate CI-only executable: any attempted yt-dlp launch writes a failure marker.
use std::{fs::OpenOptions,io::Write,path::PathBuf};
fn main(){
  let hosted=std::env::var("GITHUB_ACTIONS").ok().as_deref()==Some("true")
   && std::env::var("RUNNER_OS").ok().as_deref()==Some("Windows")
   && std::env::var("RUNNER_ENVIRONMENT").ok().as_deref()==Some("github-hosted");
  if !hosted {std::process::exit(77);}
  let (Some(temp),Some(path))=(
    std::env::var_os("RUNNER_TEMP").map(PathBuf::from),
    std::env::var_os("AKULA_VIDEO_TOOLS_01L_CHILD_MARKER").map(PathBuf::from)
  ) else {std::process::exit(78);};
  if path.is_absolute() && path.parent()==Some(temp.as_path())
      && path.file_name().and_then(|f|f.to_str())==Some("video-tools-01l-child-executed.flag"){
    if let Ok(mut marker)=OpenOptions::new().write(true).create_new(true).open(path){
      let _=marker.write_all(b"VIDEO_TOOLS_01L_FORBIDDEN_CHILD_EXECUTED\n");
      let _=marker.sync_all();
    }
  }
  std::process::exit(79);
}
