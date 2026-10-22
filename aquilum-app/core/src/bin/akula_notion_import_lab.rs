use aquilum_core::app_core::{Core,CoreEvent};
use aquilum_core::notion_api_probe::{http::HttpReadClient,inspect_root,ProbeLimits,projection::prepare_import,publish::publish_approved};
use std::{env,path::PathBuf,sync::Arc};
use uuid::Uuid;
fn main(){if let Err(e)=run(){eprintln!("Notion import refused: {e}");std::process::exit(1)}}
fn run()->Result<(),String>{
 let a:Vec<String>=env::args().skip(1).collect();
 let (publish,root,app,workspace)=match a.as_slice(){
 [mode,id] if mode=="--preview"=>(false,id,None,None),
 [mode,id,app,workspace,approval] if mode=="--publish" && approval=="--approve-create-only"=>
   (true,id,Some(PathBuf::from(app)),Some(PathBuf::from(workspace))),
 _=>return Err("usage: --preview ROOT | --publish ROOT APP_DATA REGISTERED_WORKSPACE --approve-create-only".into())
 };
 let root=Uuid::parse_str(root).map_err(|_|"invalid Notion root UUID")?.to_string();
 let token=env::var("AKULA_NOTION_READ_TOKEN").map_err(|_|"Notion token not configured locally")?;
 let mut client=HttpReadClient::new(token).map_err(|_|"invalid Notion credentials")?;
 let read=inspect_root(&mut client,&root,ProbeLimits::default())?;
 let preview=prepare_import(&read)?;
 println!("objects={} unresolved={} can_publish={}",preview.candidates.len(),preview.unresolved.len(),preview.publish_allowed);
 if publish {
   let app=app.unwrap();let workspace=workspace.unwrap();
   if !app.is_dir(){return Err("Akula app data path must exist".into())}
   let core=Core::open(&app,Arc::new(|_:CoreEvent|{}));
   let report=publish_approved(&core,&workspace,&preview,true)?;
   println!("{}",serde_json::to_string_pretty(&report).map_err(|e|e.to_string())?);
   if !report.completed{return Err("import incomplete; inspect report".into())}
 }
 Ok(())
}
