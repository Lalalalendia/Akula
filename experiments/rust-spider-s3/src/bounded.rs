//! Approved URL spool -> fixed-capacity HTTP-to-S3 pipeline.
//! This is a laboratory transport; discovery, consent and durable leases belong to callers.
use anyhow::{bail, Context, Result};
use aws_config::{BehaviorVersion, Region};
use aws_sdk_s3::{primitives::ByteStream, Client as S3};
use reqwest::{Client as Http, Url};
use sha2::{Digest, Sha256};
use std::{env, fs::File, io::{BufRead, BufReader}, sync::{Arc,atomic::{AtomicUsize,Ordering}},time::Duration};
use tokio::{sync::{mpsc, Semaphore},task::JoinSet};

const FETCH: usize = 4;
const UPLOAD: usize = 4;
const QUEUE: usize = 8;
struct Page { name:String, body:Vec<u8> }
fn variable(k:&str)->Result<String>{env::var(k).with_context(||format!("missing {k}"))}
fn task(line:&str, root:&Url)->Result<(String,Url)> {
    if line.len()>2048 {bail!("task line too large");}
    let (name,path)=line.split_once('\t').context("task format name TAB path")?;
    if name.is_empty() || name.len()>150 || name==".." || name=="." ||
      !name.bytes().all(|b|b.is_ascii_alphanumeric() || b"._-".contains(&b)){bail!("unsafe name");}
    if !path.starts_with("/pages/") || path.contains("..") || path.contains('#') ||
        path.contains('\\') || path.starts_with("//"){bail!("unsafe path");}
    let u=root.join(path)?;
    if u.origin()!=root.origin() || !u.path().starts_with("/pages/"){bail!("outside origin");}
    Ok((name.into(),u))
}
async fn fetch(http:Http,url:Url,name:String,limit:usize)->Result<Page>{
    let mut res=http.get(url.clone()).send().await?;
    if !res.status().is_success(){bail!("HTTP {} for {url}",res.status());}
    if res.content_length().is_some_and(|n|n>limit as u64){bail!("response too large");}
    let mut body=Vec::new();
    while let Some(chunk)=res.chunk().await? {
        if chunk.len()>limit.saturating_sub(body.len()){bail!("body exceeds cap");}
        body.extend_from_slice(&chunk);
    }
    if body.is_empty(){bail!("empty body");}
    Ok(Page{name,body})
}
async fn upload(s3:S3,bucket:String,prefix:String,p:Page,acks:Arc<AtomicUsize>,crash:usize)->Result<()>{
    let key=format!("{prefix}/{}",p.name);
    let sha=hex::encode(Sha256::digest(&p.body));
    let list=s3.list_objects_v2().bucket(&bucket).prefix(&key).max_keys(2).send().await?;
    if list.contents().iter().any(|x|x.key()==Some(key.as_str())) {
        let h=s3.head_object().bucket(&bucket).key(&key).send().await?;
        if h.metadata().and_then(|m|m.get("sha256"))!=Some(&sha){bail!("different bytes for {key}");}
        return Ok(());
    }
    s3.put_object().bucket(&bucket).key(&key).if_none_match("*")
       .metadata("sha256",sha).body(ByteStream::from(p.body))
       .content_type("text/html; charset=utf-8").send().await?;
    let n=acks.fetch_add(1,Ordering::SeqCst)+1;
    if crash>0 && n>=crash {std::process::exit(87);}
    Ok(())
}
async fn sink(mut rx:mpsc::Receiver<Page>,s3:S3,bucket:String,prefix:String,crash:usize)->Result<()>{
    let acks=Arc::new(AtomicUsize::new(0));
    let mut jobs=JoinSet::new();
    while let Some(page)=rx.recv().await {
        while jobs.len()>=UPLOAD { jobs.join_next().await.context("writer missing")??; }
        let (s3,bucket,prefix,acks)=(s3.clone(),bucket.clone(),prefix.clone(),acks.clone());
        jobs.spawn(async move{upload(s3,bucket,prefix,page,acks,crash).await});
    }
    while let Some(done)=jobs.join_next().await {done??;}
    Ok(())
}
pub async fn run_from_env()->Result<()>{
    let root=Url::parse(&variable("TRIAL_ROOT")?)?;
    if !matches!(root.scheme(),"http"|"https") || !root.username().is_empty() ||
      root.password().is_some(){bail!("invalid origin");}
    let expected:usize=variable("TRIAL_PAGES")?.parse()?;
    if !(1..=1_000_000).contains(&expected){bail!("invalid page count");}
    let cap:usize=env::var("TRIAL_MAX_PAGE_BYTES").unwrap_or_else(|_|String::from("1048576")).parse()?;
    if !(128..=16*1024*1024).contains(&cap){bail!("invalid page size");}
    let prefix=variable("S3_PREFIX")?;
    if prefix.is_empty()||prefix.contains("..")||prefix.starts_with('/')||prefix.ends_with('/') {bail!("invalid S3 prefix");}
    let bucket=variable("S3_BUCKET")?;
    let endpoint=variable("S3_ENDPOINT")?;
    let crash:usize=env::var("CRASH_AFTER_PUT_N").unwrap_or_default().parse().unwrap_or(0);
    let spool=variable("TRIAL_URLS_FILE")?;
    let http=Http::builder().redirect(reqwest::redirect::Policy::none())
      .timeout(Duration::from_secs(30)).build()?;
    let shared=aws_config::defaults(BehaviorVersion::latest()).region(Region::new("us-east-1")).load().await;
    let s3=S3::from_conf(aws_sdk_s3::config::Builder::from(&shared).endpoint_url(endpoint).force_path_style(true).build());
    let file=BufReader::new(File::open(spool)?);
    let (tx,rx)=mpsc::channel::<Page>(QUEUE);
    let writer=tokio::spawn(sink(rx,s3,bucket,prefix,crash));
    let sem=Arc::new(Semaphore::new(FETCH));
    let mut jobs=JoinSet::new();
    let mut count=0;
    let mut problem:Option<anyhow::Error>=None;
    for row in file.lines(){
        let parsed=row.map_err(anyhow::Error::from).and_then(|line|task(&line,&root));
        let (name,url)=match parsed {Ok(p)=>p,Err(e)=>{problem=Some(e);break;}};
        count+=1;
        if count>expected {problem=Some(anyhow::anyhow!("too many tasks"));break;}
        let permit=sem.clone().acquire_owned().await?;
        let (http,sender)=(http.clone(),tx.clone());
        jobs.spawn(async move{
            let _permit=permit;
            let page=fetch(http,url,name,cap).await?;
            sender.send(page).await.context("writer closed")?;
            Ok::<(),anyhow::Error>(())
        });
        if jobs.len()>=FETCH {
            if let Some(done)=jobs.join_next().await {
                if let Err(e)=done.context("fetch worker panic").and_then(|x|x) {
                   problem=Some(e);break;
                }
            }
        }
    }
    drop(tx);
    if problem.is_some(){jobs.abort_all();}
    while let Some(done)=jobs.join_next().await {
       if let Err(e)=done.context("fetch join error").and_then(|x|x) { if problem.is_none(){problem=Some(e);} }
    }
    if count!=expected && problem.is_none(){problem=Some(anyhow::anyhow!("count mismatch {count}/{expected}"));}
    let sink_result=writer.await.context("writer panic")?;
    if let Some(e)=problem {return Err(e);}
    sink_result?;
    println!("BOUNDED_SUCCESS pages={count} queue={QUEUE} fetch={FETCH} upload={UPLOAD}");
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn approve(){
      let root=Url::parse("http://127.0.0.1:8080/index.html").unwrap();
      assert!(task("001.html\t/pages/001.html",&root).is_ok());
      for s in ["../x\t/pages/a","x\thttp://x.invalid","x\t//x.invalid","x\t/pages/../../admin","x\t/admin","x\t/pages/x#y"]{
        assert!(task(s,&root).is_err(),"{s}");
      }
    }
}
